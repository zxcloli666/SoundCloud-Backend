use sqlx::PgPool;

use crate::queue::{JobError, JobResult};

const CONFIRM_BATCH: i64 = 1_000;

pub struct CatalogCreditHandler {
    pool: PgPool,
}

impl CatalogCreditHandler {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub async fn review(&self) -> JobResult {
        let confirmed = sqlx::query_file_scalar!(
            "queries/catalog/confirm_credits_by_identity.sql",
            CONFIRM_BATCH
        )
        .fetch_all(&self.pool)
        .await
        .map_err(JobError::retryable)?;

        if !confirmed.is_empty() {
            tracing::info!(
                credits = confirmed.len(),
                "weak track credits confirmed by a verified uploader identity"
            );
        }
        Ok(())
    }
}
