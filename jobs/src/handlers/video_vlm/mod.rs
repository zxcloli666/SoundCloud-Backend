mod queue;
mod result;
#[cfg(test)]
mod tests;

use std::sync::Arc;
use std::time::Duration;

use backend_contracts::pipeline::{VIDEO_VLM, VideoVlmRequest};
use sqlx::PgPool;
use tokio::time::MissedTickBehavior;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};
use url::Url;

use self::queue::Claimed;
use crate::bus::Bus;
use crate::config::{VideoFindConfig, VideoVlmConfig};
use crate::handlers::video_find::Pacing;
use crate::handlers::video_find::parts::Parts;
use crate::objects::Blobs;
use crate::queue::{JobError, JobResult};

const RETRY_PUBLISH: Duration = Duration::from_secs(120);

pub struct VideoVlm<B> {
    pool: PgPool,
    storage_url: Url,
    config: VideoVlmConfig,
    pacing: Pacing,
    blobs: B,
    parts: Parts,
}

impl<B: Blobs + 'static> VideoVlm<B> {
    pub fn new(
        pool: PgPool,
        storage_url: Url,
        config: VideoVlmConfig,
        find: &VideoFindConfig,
        blobs: B,
    ) -> Self {
        Self {
            pool,
            storage_url,
            config,
            pacing: Pacing {
                retry_base: find.retry_base,
                max_attempts: find.max_attempts,
                none_recheck: find.none_recheck,
                static_recheck: find.static_recheck,
            },
            blobs,
            parts: Parts::default(),
        }
    }

    pub async fn sweep(
        self: Arc<Self>,
        bus: Bus,
        cancellation: CancellationToken,
    ) -> anyhow::Result<()> {
        if self.config.in_flight <= 0 {
            info!("video VLM sweep is off: VIDEO_VLM_IN_FLIGHT is 0");
            cancellation.cancelled().await;
            return Ok(());
        }
        info!(
            in_flight = self.config.in_flight,
            tick_s = self.config.tick.as_secs(),
            "video VLM sweep started"
        );
        let mut ticker = tokio::time::interval(self.config.tick);
        ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                () = cancellation.cancelled() => return Ok(()),
                _ = ticker.tick() => {}
            }
            if let Err(error) = self.dispatch(&bus, &cancellation).await {
                warn!(%error, "video VLM dispatch failed; it resumes on the next tick");
            }
        }
    }

    async fn dispatch(&self, bus: &Bus, cancellation: &CancellationToken) -> JobResult {
        queue::requeue_unanswered(&self.pool, self.config.timeout).await?;
        let free = self.config.in_flight - queue::busy(&self.pool).await?;
        if free <= 0 {
            return Ok(());
        }
        let claimed = queue::claim(&self.pool, free).await?;
        if !claimed.is_empty() {
            debug!(claimed = claimed.len(), "video VLM requests claimed");
        }
        for claim in claimed {
            if cancellation.is_cancelled() {
                self.release(&claim, Duration::ZERO, "shutdown").await;
                continue;
            }
            if let Err(error) = self.send(bus, &claim).await {
                warn!(track = %claim.sc_track_id, %error, "video VLM request was not published");
                self.release(&claim, RETRY_PUBLISH, "publish_failed").await;
            }
        }
        Ok(())
    }

    async fn send(&self, bus: &Bus, claim: &Claimed) -> JobResult {
        let Some(held) = &claim.held else {
            self.drop_unreadable(claim).await;
            return Ok(());
        };
        let request = self.request(claim, &held.s3_key, held.offset_ms)?;
        bus.publish_dedup(VIDEO_VLM, &request, &claim.message_id(held))
            .await
            .map_err(JobError::retryable)
    }

    fn request(&self, claim: &Claimed, key: &str, offset_ms: i32) -> JobResult<VideoVlmRequest> {
        let url = self.storage_url.join(key).map_err(JobError::permanent)?;
        Ok(VideoVlmRequest {
            sc_track_id: claim.sc_track_id.clone(),
            attempt: i64::from(claim.attempt),
            video_url: url.to_string(),
            duration_ms: claim.duration_ms.filter(|duration| *duration > 0),
            offset_ms: Some(offset_ms),
        })
    }

    async fn release(&self, claim: &Claimed, wait: Duration, why: &str) {
        queue::release(&self.pool, &claim.sc_track_id, claim.attempt, wait, why).await;
    }

    async fn drop_unreadable(&self, claim: &Claimed) {
        warn!(track = %claim.sc_track_id, "held video payload is unreadable; the candidate is dropped");
        let settled = queue::settle(
            &self.pool,
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
