mod queue;
mod result;
mod sources;
#[cfg(test)]
mod tests;

use std::sync::Arc;
use std::time::Duration;

use backend_contracts::admin_video::VideoSource;
use backend_contracts::pipeline::{
    VIDEO_FIND, VideoCandidate, VideoCandidateKind, VideoFindRequest, VideoFindResult,
};
use sqlx::PgPool;
use tokio::time::MissedTickBehavior;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};
use url::Url;

use crate::bus::Bus;
use crate::config::VideoFindConfig;
use crate::handlers::indexing::canonical_storage_url;
use crate::handlers::video_accounts::{Fetch, SpotifyKey, store};
use crate::queue::{JobError, JobResult};

use self::queue::Claimed;
use self::sources::{Failure, Sources, Wanted};

const RETRY_NO_ACCOUNT: Duration = Duration::from_secs(60);
const RETRY_PUBLISH: Duration = Duration::from_secs(120);

pub(crate) struct Pacing {
    pub(crate) account_pause: Duration,
    pub(crate) retry_base: Duration,
    pub(crate) max_attempts: u32,
    pub(crate) none_recheck: Duration,
    pub(crate) static_recheck: Duration,
}

pub struct VideoFind<F> {
    pool: PgPool,
    bus: Bus,
    storage_url: Url,
    config: VideoFindConfig,
    pacing: Pacing,
    sources: Sources<F>,
}

impl<F: Fetch + 'static> VideoFind<F> {
    pub fn new(
        pool: PgPool,
        bus: Bus,
        storage_url: Url,
        config: VideoFindConfig,
        fetch: F,
        spotify: SpotifyKey,
    ) -> Self {
        let pacing = Pacing {
            account_pause: config.account_pause,
            retry_base: config.retry_base,
            max_attempts: config.max_attempts,
            none_recheck: config.none_recheck,
            static_recheck: config.static_recheck,
        };
        Self {
            pool,
            bus,
            storage_url,
            config,
            pacing,
            sources: Sources::new(fetch, spotify),
        }
    }

    pub async fn sweep(self: Arc<Self>, cancellation: CancellationToken) -> anyhow::Result<()> {
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
        loop {
            tokio::select! {
                () = cancellation.cancelled() => return Ok(()),
                _ = ticker.tick() => {}
            }
            if let Err(error) = self.dispatch(&cancellation).await {
                warn!(%error, "video dispatch failed; it resumes on the next tick");
            }
        }
    }

    pub async fn finish(&self, result: VideoFindResult) -> JobResult {
        result::record(&self.pool, &self.pacing, &result).await
    }

    async fn dispatch(&self, cancellation: &CancellationToken) -> JobResult {
        queue::requeue_unanswered(&self.pool, self.config.timeout).await?;
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
            if cancellation.is_cancelled() {
                queue::release(&self.pool, &claim, Duration::ZERO, "shutdown").await;
                continue;
            }
            if !self.send(&claim).await? {
                queue::release(&self.pool, &claim, RETRY_NO_ACCOUNT, "no_account").await;
                for rest in waiting.by_ref() {
                    queue::release(&self.pool, &rest, RETRY_NO_ACCOUNT, "no_account").await;
                }
            }
        }
        Ok(())
    }

    async fn send(&self, claim: &Claimed) -> JobResult<bool> {
        let Some((account, cookies)) = store::lease(&self.pool, "youtube")
            .await
            .map_err(JobError::retryable)?
        else {
            return Ok(false);
        };
        queue::mark_account(&self.pool, claim, &account).await?;
        let wanted = Wanted {
            artist: claim.artist.clone(),
            title: claim.title.clone(),
            isrc: claim.isrc.clone(),
            duration_ms: claim.duration_ms,
        };
        let mut candidates = youtube_searches(&wanted);
        candidates.extend(self.other_sources(&wanted).await);
        let request = self.request(claim, &wanted, candidates, account, cookies)?;
        if let Err(error) = self
            .bus
            .publish_dedup(VIDEO_FIND, &request, &claim.message_id())
            .await
        {
            warn!(track = %claim.sc_track_id, %error, "video request was not published");
            queue::release(&self.pool, claim, RETRY_PUBLISH, "publish_failed").await;
        }
        Ok(true)
    }

    fn request(
        &self,
        claim: &Claimed,
        wanted: &Wanted,
        candidates: Vec<VideoCandidate>,
        account: String,
        cookies: String,
    ) -> JobResult<VideoFindRequest> {
        let mut upload = self.config.upload_url.clone();
        upload
            .path_segments_mut()
            .map_err(|()| JobError::permanent(anyhow::anyhow!("upload URL cannot be a base")))?
            .extend([
                "video",
                &format!("{}-{}.mp4", claim.sc_track_id, claim.attempt),
            ]);
        Ok(VideoFindRequest {
            sc_track_id: claim.sc_track_id.clone(),
            attempt: i64::from(claim.attempt),
            s3_url: canonical_storage_url(&self.storage_url, &claim.sc_track_id)
                .map_err(JobError::permanent)?,
            upload_url: upload.to_string(),
            artist: wanted.artist.clone(),
            title: wanted.title.clone(),
            isrc: wanted.isrc.clone(),
            duration_ms: wanted.duration_ms,
            candidates,
            youtube_account: Some(account),
            youtube_cookies: Some(cookies),
        })
    }

    async fn other_sources(&self, wanted: &Wanted) -> Vec<VideoCandidate> {
        let mut found = Vec::new();
        if let Some((account, secret)) = self.lease("spotify").await {
            let answer = self.sources.canvas(&secret, wanted).await;
            found.extend(self.settled("spotify", &account, answer).await.flatten());
        }
        if let Some((account, secret)) = self.lease("yandex").await {
            let answer = self.sources.clips(&secret, wanted).await;
            found.extend(
                self.settled("yandex", &account, answer)
                    .await
                    .unwrap_or_default(),
            );
        }
        found
    }

    async fn lease(&self, source: &str) -> Option<(String, String)> {
        match store::lease(&self.pool, source).await {
            Ok(Some(leased)) => {
                tokio::time::sleep(self.config.source_gap).await;
                Some(leased)
            }
            Ok(None) => None,
            Err(error) => {
                warn!(source, %error, "video account lease failed");
                None
            }
        }
    }

    async fn settled<T>(
        &self,
        source: &str,
        account: &str,
        answer: Result<T, Failure>,
    ) -> Option<T> {
        match answer {
            Ok(value) => Some(value),
            Err(Failure::Paused(error)) => {
                self.account_out(account, &error, Some(self.config.account_pause))
                    .await;
                None
            }
            Err(Failure::Rejected(error)) => {
                self.account_out(account, &error, None).await;
                None
            }
            Err(Failure::Unavailable(error)) => {
                warn!(source, account, %error, "video source is unavailable");
                None
            }
        }
    }

    async fn account_out(&self, account: &str, error: &str, pause: Option<Duration>) {
        let outcome = match pause {
            Some(pause) => store::pause(&self.pool, account, pause, error).await,
            None => store::kill(&self.pool, account, error).await,
        };
        match outcome {
            Ok(()) => warn!(account, error, "video account taken out of rotation"),
            Err(failure) => warn!(account, %failure, "video account state was not saved"),
        }
    }
}

fn youtube_searches(wanted: &Wanted) -> Vec<VideoCandidate> {
    let base = format!("{} {}", wanted.artist, wanted.title);
    [format!("{base} official video"), base]
        .into_iter()
        .map(|query| VideoCandidate {
            source: VideoSource::Youtube,
            kind: VideoCandidateKind::Search,
            value: query,
            trusted: false,
            source_ref: None,
        })
        .collect()
}
