mod fetch;
mod leads;
pub(crate) mod parts;
mod pick;
mod pool;
mod queue;
mod result;
mod sources;
mod stage;
mod temp;
#[cfg(test)]
pub(crate) mod tests;
pub(crate) mod ytdlp;

use std::sync::Arc;
use std::time::{Duration, Instant};

use backend_contracts::pipeline::{VIDEO_FIND, VideoFindRequest};
use backend_contracts::reasons::WorkerReason;
use sqlx::PgPool;
use tokio::time::MissedTickBehavior;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};
use url::Url;

use crate::bus::Bus;
use crate::config::VideoFindConfig;
use crate::handlers::indexing::canonical_storage_url;
use crate::handlers::video_accounts::{Fetch, Seal, SpotifyKey};
use crate::objects::Blobs;
use crate::queue::{JobError, JobResult};

pub(crate) use self::fetch::Pooled;
use self::parts::Parts;
pub(crate) use self::pool::ProxyPool;
use self::queue::Claimed;
pub(crate) use self::result::final_key;
use self::sources::{Sources, Wanted};
use self::ytdlp::Tube;

const RETRY_NO_ACCOUNT: Duration = Duration::from_secs(60);
const RETRY_PUBLISH: Duration = Duration::from_secs(120);
const POOL_EVERY: Duration = Duration::from_secs(600);
const UNSTAGED_SLACK: Duration = Duration::from_secs(300);

pub(crate) struct Pacing {
    pub(crate) retry_base: Duration,
    pub(crate) max_attempts: u32,
    pub(crate) none_recheck: Duration,
    pub(crate) static_recheck: Duration,
}

pub struct VideoFind<F, B, T> {
    pool: PgPool,
    storage_url: Url,
    config: VideoFindConfig,
    pacing: Pacing,
    sources: Sources<F>,
    blobs: B,
    tube: T,
    seal: Seal,
    parts: Parts,
}

impl<F: Fetch + 'static, B: Blobs + 'static, T: Tube + 'static> VideoFind<F, B, T> {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        pool: PgPool,
        storage_url: Url,
        config: VideoFindConfig,
        fetch: F,
        spotify: SpotifyKey,
        blobs: B,
        tube: T,
        seal: Seal,
    ) -> Self {
        let pacing = Pacing {
            retry_base: config.retry_base,
            max_attempts: config.max_attempts,
            none_recheck: config.none_recheck,
            static_recheck: config.static_recheck,
        };
        Self {
            pool,
            storage_url,
            config,
            pacing,
            sources: Sources::new(fetch, spotify),
            blobs,
            tube,
            seal,
            parts: Parts::default(),
        }
    }

    pub async fn sweep(
        self: Arc<Self>,
        bus: Bus,
        cancellation: CancellationToken,
    ) -> anyhow::Result<()> {
        if self.config.in_flight <= 0 {
            info!("video sweep is off: VIDEO_FIND_IN_FLIGHT is 0");
            cancellation.cancelled().await;
            return Ok(());
        }
        info!(
            in_flight = self.config.in_flight,
            tick_s = self.config.tick.as_secs(),
            "video sweep started"
        );
        let mut ticker = tokio::time::interval(self.config.tick);
        ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
        let mut kept: Option<Instant> = None;
        loop {
            tokio::select! {
                () = cancellation.cancelled() => return Ok(()),
                _ = ticker.tick() => {}
            }
            if kept.is_none_or(|at| at.elapsed() >= POOL_EVERY) {
                match queue::keep_pool(&self.pool).await {
                    Ok(()) => kept = Some(Instant::now()),
                    Err(error) => {
                        warn!(%error, "video pool was not kept; it resumes on the next tick");
                        continue;
                    }
                }
            }
            if let Err(error) = self.dispatch(&bus, &cancellation).await {
                warn!(%error, "video dispatch failed; it resumes on the next tick");
            }
        }
    }

    async fn dispatch(&self, bus: &Bus, cancellation: &CancellationToken) -> JobResult {
        let unstaged = self.config.stage_timeout + UNSTAGED_SLACK;
        queue::requeue_unanswered(&self.pool, self.config.timeout, unstaged).await?;
        let free = self.config.in_flight - queue::busy(&self.pool).await?;
        if free <= 0 {
            return Ok(());
        }
        let claimed = queue::claim(&self.pool, free).await?;
        if !claimed.is_empty() {
            debug!(claimed = claimed.len(), "video requests claimed");
        }
        let mut waiting = claimed.into_iter();
        while let Some(claim) = waiting.next() {
            let sent = tokio::select! {
                biased;
                () = cancellation.cancelled() => {
                    queue::release(&self.pool, &claim, Duration::ZERO, "shutdown").await;
                    continue;
                }
                sent = tokio::time::timeout(self.config.stage_timeout, self.send(bus, &claim)) => sent,
            };
            let Ok(sent) = sent else {
                warn!(track = %claim.sc_track_id, "video candidates took too long to stage");
                queue::release(&self.pool, &claim, RETRY_PUBLISH, "stage_timeout").await;
                continue;
            };
            match sent {
                Ok(true) => {}
                Ok(false) => {
                    queue::release(&self.pool, &claim, RETRY_NO_ACCOUNT, "no_account").await;
                    for rest in waiting.by_ref() {
                        queue::release(&self.pool, &rest, RETRY_NO_ACCOUNT, "no_account").await;
                    }
                }
                Err(error) => {
                    warn!(track = %claim.sc_track_id, %error, "video candidates were not staged");
                    queue::release(&self.pool, &claim, RETRY_PUBLISH, "stage_failed").await;
                }
            }
        }
        Ok(())
    }

    async fn send(&self, bus: &Bus, claim: &Claimed) -> JobResult<bool> {
        let Some(youtube) = self.lease_youtube().await.map_err(JobError::retryable)? else {
            return Ok(false);
        };
        let wanted = Wanted {
            artist: claim.artist.clone(),
            title: claim.title.clone(),
            isrc: claim.isrc.clone(),
            duration_ms: claim.duration_ms,
        };
        let leads = self.leads(&wanted, &youtube).await;
        let staged = self.bring_all(claim, leads, &youtube).await?;
        if staged.candidates.is_empty() {
            result::settle(
                &self.pool,
                &claim.sc_track_id,
                claim.attempt,
                self.pacing.none_recheck,
                WorkerReason::NoVideoFound,
            )
            .await?;
            return Ok(true);
        }
        if !queue::stage(&self.pool, claim, &staged).await? {
            self.drop_temps(&staged.keys()).await;
            return Ok(true);
        }
        let request = self.request(claim, &wanted, &staged)?;
        if let Err(error) = bus
            .publish_dedup(VIDEO_FIND, &request, &claim.message_id())
            .await
        {
            warn!(track = %claim.sc_track_id, %error, "video request was not published");
            queue::release(&self.pool, claim, RETRY_PUBLISH, "publish_failed").await;
            self.drop_temps(&staged.keys()).await;
        }
        Ok(true)
    }

    fn request(
        &self,
        claim: &Claimed,
        wanted: &Wanted,
        staged: &stage::Staged,
    ) -> JobResult<VideoFindRequest> {
        Ok(VideoFindRequest {
            sc_track_id: claim.sc_track_id.clone(),
            attempt: i64::from(claim.attempt),
            s3_url: canonical_storage_url(&self.storage_url, &claim.sc_track_id)
                .map_err(JobError::permanent)?,
            artist: wanted.artist.clone(),
            title: wanted.title.clone(),
            duration_ms: wanted.duration_ms,
            candidates: self.candidate_list(staged)?,
        })
    }
}
