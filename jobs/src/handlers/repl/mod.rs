mod replay;
mod retention;
mod status;

use std::sync::Mutex;
use std::time::Instant;

use backend_contracts::ReplReplayPayload;
use sqlx::PgPool;

use crate::queue::JobResult;

pub struct ReplHandler {
    pool: PgPool,
    status_backfilled_at: Mutex<Option<Instant>>,
}

impl ReplHandler {
    pub fn new(pool: PgPool) -> Self {
        Self {
            pool,
            status_backfilled_at: Mutex::new(None),
        }
    }

    pub async fn retain(&self) -> JobResult {
        retention::run(&self.pool).await?;
        status::reconcile(&self.pool, &self.status_backfilled_at).await
    }

    pub async fn replay(&self, payload: ReplReplayPayload) -> JobResult {
        replay::run(&self.pool, payload).await
    }
}

#[cfg(test)]
mod tests;
