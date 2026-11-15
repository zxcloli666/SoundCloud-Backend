#[cfg(test)]
mod live_tests;
mod pass;
mod queue;
mod result;

use std::sync::Arc;
use std::time::{Duration, Instant};

use backend_contracts::pipeline::{
    HIGHLIGHT_TRACK, HIGHLIGHT_VERSION, HighlightRequest, HighlightResult,
};
use sqlx::PgPool;
use tokio::time::MissedTickBehavior;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};
use url::Url;

use crate::bus::Bus;
use crate::config::HighlightConfig;
use crate::handlers::indexing::canonical_storage_url;
use crate::queue::{JobError, JobResult};

use self::pass::Pass;
use self::queue::Claimed;

const TICK: Duration = Duration::from_secs(15);

pub struct HighlightHandler {
    pool: PgPool,
    bus: Bus,
    storage_url: Url,
    config: HighlightConfig,
}

impl HighlightHandler {
    pub fn new(pool: PgPool, bus: Bus, storage_url: Url, config: HighlightConfig) -> Self {
        Self {
            pool,
            bus,
            storage_url,
            config,
        }
    }

    pub async fn sweep(self: Arc<Self>, cancellation: CancellationToken) -> anyhow::Result<()> {
        if self.config.in_flight <= 0 {
            info!("highlight sweep is off: HIGHLIGHT_IN_FLIGHT is 0");
            cancellation.cancelled().await;
            return Ok(());
        }
        info!(
            in_flight = self.config.in_flight,
            walk_batch = self.config.walk_batch,
            rewalk_s = self.config.rewalk.as_secs(),
            version = HIGHLIGHT_VERSION,
            "highlight sweep started"
        );
        let mut pass = Pass::new(self.config.rewalk);
        let mut ticker = tokio::time::interval(TICK);
        ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                () = cancellation.cancelled() => return Ok(()),
                _ = ticker.tick() => {}
            }
            if pass.due(Instant::now()) {
                match queue::walk(&self.pool, pass.after(), self.config.walk_batch).await {
                    Ok(next) => pass.advance(next, Instant::now()),
                    Err(error) => {
                        warn!(%error, "highlight walk failed; it resumes on the next tick")
                    }
                }
            }
            if let Err(error) = self.dispatch().await {
                warn!(%error, "highlight dispatch failed; it resumes on the next tick");
            }
        }
    }

    pub async fn finish(&self, result: HighlightResult) -> JobResult {
        result::record(&self.pool, self.config.max_failures, &result).await
    }

    async fn dispatch(&self) -> JobResult {
        let claimed = queue::claim(&self.pool, self.config.in_flight, self.config.timeout).await?;
        if !claimed.is_empty() {
            debug!(claimed = claimed.len(), "highlight requests claimed");
        }
        let mut first_failure = None;
        for claim in claimed {
            if let Err(error) = self.publish(&claim).await {
                queue::release(&self.pool, &claim).await;
                first_failure.get_or_insert(error);
            }
        }
        first_failure.map_or(Ok(()), Err)
    }

    async fn publish(&self, claim: &Claimed) -> JobResult {
        let request = HighlightRequest {
            s3_url: canonical_storage_url(&self.storage_url, &claim.sc_track_id)
                .map_err(JobError::permanent)?,
            sc_track_id: claim.sc_track_id.clone(),
            attempt: i64::from(claim.attempt),
            version: claim.version,
        };
        self.bus
            .publish_dedup(HIGHLIGHT_TRACK, &request, &claim.message_id())
            .await
            .map_err(JobError::retryable)
    }
}
