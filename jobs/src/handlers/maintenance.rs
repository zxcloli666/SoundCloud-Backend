use std::sync::Mutex;
use std::time::{Duration, Instant};

use sqlx::PgPool;

use crate::queue::{JobError, JobResult};

const CLEANUP_BATCH: i64 = 1_000;
const RECEIPTS_FULL_SWEEP_EVERY: Duration = Duration::from_secs(24 * 60 * 60);

pub struct MaintenanceHandler {
    pool: PgPool,
    receipts_swept_at: Mutex<Option<Instant>>,
}

impl MaintenanceHandler {
    pub fn new(pool: PgPool) -> Self {
        Self {
            pool,
            receipts_swept_at: Mutex::new(None),
        }
    }

    pub async fn cleanup_login_requests(&self) -> JobResult {
        sqlx::query_file!(
            "queries/maintenance/cleanup_login_requests.sql",
            CLEANUP_BATCH
        )
        .execute(&self.pool)
        .await
        .map_err(JobError::retryable)?;
        Ok(())
    }

    pub async fn cleanup_diag(&self) -> JobResult {
        loop {
            let bundles = sqlx::query_file!(
                "queries/maintenance/cleanup_diag_bundles.sql",
                CLEANUP_BATCH
            )
            .execute(&self.pool)
            .await
            .map_err(JobError::retryable)?;
            let events =
                sqlx::query_file!("queries/maintenance/cleanup_diag_events.sql", CLEANUP_BATCH)
                    .execute(&self.pool)
                    .await
                    .map_err(JobError::retryable)?;
            let days = sqlx::query_file!(
                "queries/maintenance/cleanup_diag_issue_days.sql",
                CLEANUP_BATCH
            )
            .execute(&self.pool)
            .await
            .map_err(JobError::retryable)?;
            let origins = sqlx::query_file!(
                "queries/maintenance/cleanup_diag_issue_origins.sql",
                CLEANUP_BATCH
            )
            .execute(&self.pool)
            .await
            .map_err(JobError::retryable)?;
            let removed = bundles.rows_affected()
                + events.rows_affected()
                + days.rows_affected()
                + origins.rows_affected();
            if removed == 0 {
                return Ok(());
            }
        }
    }

    pub async fn cleanup_link_requests(&self) -> JobResult {
        sqlx::query_file!(
            "queries/maintenance/cleanup_link_requests.sql",
            CLEANUP_BATCH
        )
        .execute(&self.pool)
        .await
        .map_err(JobError::retryable)?;
        Ok(())
    }

    pub async fn cleanup_job_receipts(&self) -> JobResult {
        let full_sweep = self.receipts_swept_at.lock().map_or(true, |swept_at| {
            swept_at.is_none_or(|at| at.elapsed() >= RECEIPTS_FULL_SWEEP_EVERY)
        });
        let mut transaction = self.pool.begin().await.map_err(JobError::retryable)?;
        sqlx::query_file!("queries/maintenance/cleanup_job_failures.sql")
            .execute(&mut *transaction)
            .await
            .map_err(JobError::retryable)?;
        sqlx::query_file!("queries/maintenance/cleanup_job_receipts.sql", full_sweep)
            .execute(&mut *transaction)
            .await
            .map_err(JobError::retryable)?;
        sqlx::query_file!("queries/maintenance/cleanup_pipeline_event_receipts.sql")
            .execute(&mut *transaction)
            .await
            .map_err(JobError::retryable)?;
        sqlx::query_file!("queries/enrich/ai/prune_cache.sql")
            .execute(&mut *transaction)
            .await
            .map_err(JobError::retryable)?;
        transaction.commit().await.map_err(JobError::retryable)?;
        if full_sweep && let Ok(mut swept_at) = self.receipts_swept_at.lock() {
            *swept_at = Some(Instant::now());
        }
        Ok(())
    }

    pub async fn heal_sync_queue(&self) -> JobResult {
        let mut transaction = self.pool.begin().await.map_err(JobError::retryable)?;
        sqlx::query_file!("queries/maintenance/heal_track_likes.sql")
            .execute(&mut *transaction)
            .await
            .map_err(JobError::retryable)?;
        sqlx::query_file!("queries/maintenance/heal_playlist_likes.sql")
            .execute(&mut *transaction)
            .await
            .map_err(JobError::retryable)?;
        sqlx::query_file!("queries/maintenance/heal_followings.sql")
            .execute(&mut *transaction)
            .await
            .map_err(JobError::retryable)?;
        transaction.commit().await.map_err(JobError::retryable)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use sqlx::PgPool;

    async fn install_cache(pool: &PgPool) -> anyhow::Result<()> {
        sqlx::raw_sql(
            "CREATE TABLE ai_resolver_cache (
                 cache_key text PRIMARY KEY,
                 reply jsonb NOT NULL,
                 expires_at timestamptz NOT NULL
             );
             INSERT INTO ai_resolver_cache (cache_key, reply, expires_at) VALUES
                 ('stale', '{}'::jsonb, now() - interval '1 day'),
                 ('fresh', '{}'::jsonb, now() + interval '1 day')",
        )
        .execute(pool)
        .await?;
        Ok(())
    }

    async fn remaining(pool: &PgPool) -> anyhow::Result<Vec<String>> {
        Ok(
            sqlx::query_scalar("SELECT cache_key FROM ai_resolver_cache ORDER BY cache_key")
                .fetch_all(pool)
                .await?,
        )
    }

    #[sqlx::test(migrations = false)]
    async fn an_answer_nobody_can_use_anymore_stops_taking_up_room(
        pool: PgPool,
    ) -> anyhow::Result<()> {
        install_cache(&pool).await?;

        sqlx::query_file!("queries/enrich/ai/prune_cache.sql")
            .execute(&pool)
            .await?;

        assert_eq!(
            remaining(&pool).await?,
            vec!["fresh".to_owned()],
            "the read path already refuses an expired answer, so nothing ever deleted one and \
             the table only grew"
        );
        Ok(())
    }
}

#[cfg(test)]
#[path = "maintenance_diag_tests.rs"]
mod diag_tests;
