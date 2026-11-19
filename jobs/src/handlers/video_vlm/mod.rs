mod frames;
pub mod haiku;
pub mod prompt;
mod queue;
mod result;
#[cfg(test)]
mod tests;

use std::sync::Arc;
use std::time::Duration;

use backend_contracts::pipeline::{VIDEO_VLM, VideoLabel, VideoVlmRequest};
use chrono::Utc;
use sqlx::PgPool;
use tokio::time::MissedTickBehavior;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};
use url::Url;

use self::haiku::Vision;
use self::queue::{Claimed, Held};
use crate::config::{VideoFindConfig, VideoVlmConfig};
use crate::handlers::video_accounts::Notify;
use crate::handlers::video_find::Pacing;
use crate::handlers::video_find::parts::Parts;
use crate::handlers::video_gen::Lanes;
use crate::llm::ledger;
use crate::objects::Blobs;
use crate::queue::{JobError, JobResult};

pub const LADDER: &str = "video_vlm";

const RETRY_PUBLISH: Duration = Duration::from_secs(120);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Step {
    Classify,
    Frames,
    Cut(VideoLabel),
}

pub struct VideoVlm<B, L, V, N> {
    pool: PgPool,
    storage_url: Url,
    config: VideoVlmConfig,
    pacing: Pacing,
    blobs: B,
    parts: Parts,
    lanes: L,
    haiku: Option<V>,
    notify: N,
    reset_day: u32,
}

impl<B: Blobs + 'static, L: Lanes, V: Vision, N: Notify> VideoVlm<B, L, V, N> {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        pool: PgPool,
        storage_url: Url,
        config: VideoVlmConfig,
        find: &VideoFindConfig,
        blobs: B,
        lanes: L,
        haiku: Option<V>,
        notify: N,
    ) -> Self {
        let reset_day = config.haiku.as_ref().map_or(1, |ladder| ladder.reset_day);
        Self {
            pool,
            storage_url,
            config,
            pacing: Pacing::of(find),
            blobs,
            parts: Parts::default(),
            lanes,
            haiku,
            notify,
            reset_day,
        }
    }

    pub async fn sweep(self: Arc<Self>, cancellation: CancellationToken) -> anyhow::Result<()> {
        if self.config.in_flight <= 0 {
            info!("video VLM sweep is off: VIDEO_VLM_IN_FLIGHT is 0");
            cancellation.cancelled().await;
            return Ok(());
        }
        info!(
            in_flight = self.config.in_flight,
            tick_s = self.config.tick.as_secs(),
            haiku = self.haiku.is_some(),
            month_usd = self.config.month_usd,
            "video VLM sweep started"
        );
        let mut ticker = tokio::time::interval(self.config.tick);
        ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                () = cancellation.cancelled() => return Ok(()),
                _ = ticker.tick() => {}
            }
            if let Err(error) = self.dispatch(&cancellation).await {
                warn!(%error, "video VLM dispatch failed; it resumes on the next tick");
            }
        }
    }

    pub(crate) async fn dispatch(&self, cancellation: &CancellationToken) -> JobResult {
        queue::requeue_unanswered(&self.pool, self.config.timeout).await?;
        let free = self.config.in_flight - queue::busy(&self.pool).await?;
        if free <= 0 {
            return Ok(());
        }
        let claimed = queue::claim(&self.pool, free).await?;
        if claimed.is_empty() {
            return Ok(());
        }
        debug!(claimed = claimed.len(), "video VLM requests claimed");
        let open = self.haiku_open().await?;
        for claim in claimed {
            if cancellation.is_cancelled() {
                self.release(&claim, Duration::ZERO, "shutdown").await;
                continue;
            }
            self.send(&claim, open).await;
        }
        Ok(())
    }

    async fn send(&self, claim: &Claimed, open: bool) {
        let Some(held) = &claim.held else {
            self.drop_unreadable(claim).await;
            return;
        };
        let step = match &held.haiku {
            Some(known) if known.label != VideoLabel::Static => Step::Cut(known.label),
            _ if open => Step::Frames,
            _ => Step::Classify,
        };
        if let Err(error) = self.publish(claim, held, step).await {
            warn!(track = %claim.sc_track_id, %error, "video VLM request was not published");
            self.release(claim, RETRY_PUBLISH, "publish_failed").await;
        }
    }

    async fn publish(&self, claim: &Claimed, held: &Held, step: Step) -> JobResult {
        let request = self.request(claim, held, step)?;
        let payload = serde_json::to_value(&request).map_err(JobError::permanent)?;
        self.lanes
            .publish(VIDEO_VLM, payload, claim.message_id(held))
            .await
            .map_err(JobError::retryable)
    }

    fn request(&self, claim: &Claimed, held: &Held, step: Step) -> JobResult<VideoVlmRequest> {
        let url = self
            .storage_url
            .join(&held.s3_key)
            .map_err(JobError::permanent)?;
        Ok(VideoVlmRequest {
            sc_track_id: claim.sc_track_id.clone(),
            attempt: i64::from(claim.attempt),
            video_url: url.to_string(),
            duration_ms: claim.duration_ms.filter(|duration| *duration > 0),
            offset_ms: Some(held.offset_ms),
            frames: (step == Step::Frames).then_some(true),
            label: match step {
                Step::Cut(label) => Some(label),
                _ => None,
            },
            prompt: (step == Step::Classify).then(|| prompt::local(&prompt::Features::of(held))),
        })
    }

    async fn haiku_open(&self) -> JobResult<bool> {
        if self.haiku.is_none() {
            return Ok(false);
        }
        let paused = ledger::paused(&self.pool, LADDER).await?;
        if paused.as_ref().is_some_and(|pause| !pause.due) {
            return Ok(false);
        }
        let period = ledger::period(Utc::now(), self.reset_day);
        let spent = ledger::spent(&self.pool, LADDER, period).await?;
        if spent < self.config.month_usd {
            if paused.is_some() {
                ledger::resume(&self.pool, LADDER).await?;
            }
            return Ok(true);
        }
        let text = format!(
            "Video VLM checks moved to the local model: the Haiku budget of ${:.2} for the month from {period} is spent (${spent:.2}). Haiku comes back by itself on day {} of the month.",
            self.config.month_usd, self.reset_day
        );
        ledger::pause(
            &self.pool,
            &self.notify,
            LADDER,
            ledger::BUDGET,
            self.config.recheck,
            &text,
        )
        .await?;
        Ok(false)
    }

    async fn release(&self, claim: &Claimed, wait: Duration, why: &str) {
        queue::release(&self.pool, &claim.sc_track_id, claim.attempt, wait, why).await;
    }

    async fn drop_unreadable(&self, claim: &Claimed) {
        warn!(track = %claim.sc_track_id, "held video payload is unreadable; the candidate is dropped");
        let settled = queue::settle(
            &self.pool,
            &self.pacing,
            &claim.sc_track_id,
            claim.attempt,
            "none",
            self.pacing.none_recheck,
            Some("held_unreadable"),
        )
        .await;
        if let Err(error) = settled {
            warn!(track = %claim.sc_track_id, %error, "unreadable held video was not settled");
        }
    }
}
