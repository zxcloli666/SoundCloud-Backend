mod colike;
pub(super) mod quality;
mod wave_priority;

use std::time::{Duration, Instant};

use sqlx::PgPool;
use tracing::info;

use crate::queue::{JobError, JobResult};

const COLIKE_RESUME_DELAY: Duration = Duration::from_secs(1);

pub(super) struct RecommendationHandler {
    pool: PgPool,
    wave_priority_shards: i64,
}

impl RecommendationHandler {
    pub fn new(pool: PgPool, wave_priority_shards: i64) -> Self {
        Self {
            pool,
            wave_priority_shards,
        }
    }

    pub async fn rebuild_colike(&self) -> JobResult {
        let started = Instant::now();
        let rebuilt = colike::rebuild(&self.pool).await?;
        if !rebuilt.finished {
            info!(
                batches = rebuilt.batches,
                upserted = rebuilt.upserted,
                removed = rebuilt.removed,
                elapsed_ms = started.elapsed().as_millis() as u64,
                "recommendation colike paused at its cursor"
            );
            return Err(JobError::postponed(
                COLIKE_RESUME_DELAY,
                anyhow::anyhow!("recommendation colike continues from its cursor"),
            ));
        }
        info!(
            fresh = rebuilt.fresh,
            batches = rebuilt.batches,
            upserted = rebuilt.upserted,
            removed = rebuilt.removed,
            elapsed_ms = started.elapsed().as_millis() as u64,
            "recommendation colike rebuilt"
        );
        Ok(())
    }

    pub async fn bump_wave_priority(&self) -> JobResult {
        let started = Instant::now();
        let shards = self.wave_priority_shards;
        let tracks = wave_priority::bump(&self.pool, shards).await?;
        info!(
            tracks,
            shards,
            elapsed_ms = started.elapsed().as_millis() as u64,
            "recommendation wave priorities raised"
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests;
