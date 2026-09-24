use std::time::Duration;

use futures::{StreamExt, stream};
use sqlx::PgPool;
use tracing::{info, warn};

use crate::config::DurationConfig;
use crate::queue::{JobError, JobRepository, JobResult};

use super::public_client::{PublicReadError, PublicSoundCloudClient};
use super::{new_index_job, queue_error};

const RETRY_BASE_DELAY: Duration = Duration::from_secs(2 * 60);
const RETRY_MAX_DELAY: Duration = Duration::from_secs(6 * 60 * 60);
const NOT_FOUND_RETRY_DELAY: Duration = Duration::from_secs(6 * 60 * 60);

pub struct DurationResolver {
    pool: PgPool,
    queue: JobRepository,
    client: PublicSoundCloudClient,
    batch_size: i64,
    concurrency: usize,
    max_track_duration_ms: i32,
}

enum Resolution {
    Updated,
    Cleared,
    Deferred(Option<Duration>),
}

impl DurationResolver {
    pub fn new(
        pool: PgPool,
        queue: JobRepository,
        config: &DurationConfig,
    ) -> Result<Self, crate::ClientBuildError> {
        Ok(Self {
            pool,
            queue,
            client: PublicSoundCloudClient::new(config)?,
            batch_size: config.batch_size,
            concurrency: config.concurrency,
            max_track_duration_ms: config.max_track_duration_ms,
        })
    }

    pub async fn resolve_due(&self) -> JobResult {
        let track_ids = sqlx::query_scalar::<_, String>(include_str!(
            "../../../queries/indexing/duration/select_due.sql"
        ))
        .bind(self.batch_size)
        .fetch_all(&self.pool)
        .await
        .map_err(JobError::retryable)?;
        if track_ids.is_empty() {
            return Ok(());
        }

        let mut results = stream::iter(track_ids)
            .map(|track_id| self.resolve_one(track_id))
            .buffer_unordered(self.concurrency);
        let mut updated = 0usize;
        let mut cleared = 0usize;
        let mut deferred = 0usize;
        let mut retry_after = None;
        while let Some(result) = results.next().await {
            match result? {
                Resolution::Updated => updated += 1,
                Resolution::Cleared => cleared += 1,
                Resolution::Deferred(delay) => {
                    deferred += 1;
                    retry_after = retry_after.max(delay);
                    if retry_after.is_some() {
                        break;
                    }
                }
            }
        }
        drop(results);
        if let Some(delay) = retry_after {
            self.defer_schedule(delay).await?;
        }
        info!(updated, cleared, deferred, "track durations resolved");
        Ok(())
    }

    async fn resolve_one(&self, track_id: String) -> JobResult<Resolution> {
        if track_id.is_empty() || !track_id.bytes().all(|byte| byte.is_ascii_digit()) {
            self.clear(&track_id).await?;
            return Ok(Resolution::Cleared);
        }

        let duration_ms = match self.client.track(&track_id).await {
            Ok(duration_ms) => duration_ms,
            Err(PublicReadError::NotFound) => {
                self.defer_track(&track_id, NOT_FOUND_RETRY_DELAY).await?;
                return Ok(Resolution::Deferred(None));
            }
            Err(PublicReadError::RateLimited(delay)) => {
                warn!(
                    track_id,
                    retry_seconds = delay.as_secs(),
                    "duration lookup rate limited"
                );
                self.defer_track(&track_id, delay).await?;
                return Ok(Resolution::Deferred(Some(delay)));
            }
            Err(error) => {
                warn!(track_id, %error, "duration lookup deferred");
                self.defer_track(&track_id, RETRY_BASE_DELAY).await?;
                return Ok(Resolution::Deferred(None));
            }
        };

        self.persist_duration(track_id, duration_ms).await?;
        Ok(Resolution::Updated)
    }

    async fn persist_duration(&self, track_id: String, duration_ms: i32) -> JobResult {
        let mut transaction = self.pool.begin().await.map_err(JobError::retryable)?;
        let applied = sqlx::query(include_str!("../../../queries/indexing/duration/apply.sql"))
            .bind(&track_id)
            .bind(duration_ms)
            .bind(self.max_track_duration_ms)
            .execute(&mut *transaction)
            .await
            .map_err(JobError::retryable)?;
        if applied.rows_affected() == 1 && duration_ms <= self.max_track_duration_ms {
            let job = new_index_job(track_id)?;
            self.queue
                .enqueue_in(&mut transaction, &job)
                .await
                .map_err(queue_error)?;
        }
        transaction.commit().await.map_err(JobError::retryable)?;
        Ok(())
    }

    async fn clear(&self, track_id: &str) -> JobResult {
        sqlx::query(include_str!("../../../queries/indexing/duration/clear.sql"))
            .bind(track_id)
            .execute(&self.pool)
            .await
            .map_err(JobError::retryable)?;
        Ok(())
    }

    async fn defer_schedule(&self, delay: Duration) -> JobResult {
        let milliseconds = i64::try_from(delay.as_millis()).unwrap_or(i64::MAX);
        sqlx::query_file!("queries/indexing/duration/defer_schedule.sql", milliseconds)
            .execute(&self.pool)
            .await
            .map_err(JobError::retryable)?;
        Ok(())
    }

    async fn defer_track(&self, track_id: &str, base_delay: Duration) -> JobResult {
        let base_millis = i64::try_from(base_delay.as_millis()).unwrap_or(i64::MAX);
        let max_millis = i64::try_from(RETRY_MAX_DELAY.as_millis()).unwrap_or(i64::MAX);
        sqlx::query(include_str!(
            "../../../queries/indexing/duration/defer_track.sql"
        ))
        .bind(base_millis)
        .bind(max_millis)
        .bind(track_id)
        .execute(&self.pool)
        .await
        .map_err(JobError::retryable)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> anyhow::Result<DurationConfig> {
        Ok(DurationConfig {
            api_v2_url: "http://127.0.0.1:1".parse()?,
            web_url: "http://127.0.0.1:1".parse()?,
            proxy_url: None,
            proxy_fallback: false,
            batch_size: 50,
            concurrency: 4,
            request_gap: Duration::from_millis(50),
            max_track_duration_ms: 420_000,
        })
    }

    fn resolver(pool: &PgPool) -> anyhow::Result<DurationResolver> {
        let queue = JobRepository::new(pool.clone(), "duration-tests".to_owned());
        DurationResolver::new(pool.clone(), queue, &config()?)
    }

    async fn install_schema(pool: &PgPool) -> anyhow::Result<()> {
        sqlx::raw_sql(
            "CREATE TABLE tracks (
                 sc_track_id text PRIMARY KEY,
                 duration_ms integer NOT NULL,
                 needs_duration_resolve boolean NOT NULL,
                 duration_resolve_attempts smallint NOT NULL DEFAULT 0,
                 duration_resolve_retry_at timestamptz,
                 storage_state varchar(16) NOT NULL,
                 storage_attempts smallint NOT NULL,
                 index_state varchar(16) NOT NULL,
                 indexed_at timestamptz,
                 transcribe_state varchar(16),
                 hq_upgrade_pending boolean NOT NULL DEFAULT false,
                 updated_at timestamptz NOT NULL DEFAULT now()
             );",
        )
        .execute(pool)
        .await?;
        Ok(())
    }

    #[sqlx::test(migrations = false)]
    async fn corrected_duration_revives_failed_storage(pool: PgPool) -> anyhow::Result<()> {
        install_schema(&pool).await?;
        sqlx::query(
            "INSERT INTO tracks (
                 sc_track_id, duration_ms, needs_duration_resolve,
                 storage_state, storage_attempts, index_state
             ) VALUES ('42', 30000, true, 'failed', 4, 'pending')",
        )
        .execute(&pool)
        .await?;

        sqlx::query(include_str!("../../../queries/indexing/duration/apply.sql"))
            .bind("42")
            .bind(180_000_i32)
            .bind(420_000_i32)
            .execute(&pool)
            .await?;

        let state: (i32, bool, String, i16) = sqlx::query_as(
            "SELECT duration_ms, needs_duration_resolve, storage_state, storage_attempts
             FROM tracks WHERE sc_track_id = '42'",
        )
        .fetch_one(&pool)
        .await?;
        assert_eq!(state, (180_000, false, "pending".to_owned(), 0));
        Ok(())
    }

    #[sqlx::test(migrations = false)]
    async fn oversized_tracks_become_terminal_in_one_write(pool: PgPool) -> anyhow::Result<()> {
        install_schema(&pool).await?;
        sqlx::query(
            "INSERT INTO tracks (
                 sc_track_id, duration_ms, needs_duration_resolve,
                 storage_state, storage_attempts, index_state, indexed_at
             ) VALUES ('42', 30000, true, 'pending', 0, 'indexed', now())",
        )
        .execute(&pool)
        .await?;

        sqlx::query(include_str!("../../../queries/indexing/duration/apply.sql"))
            .bind("42")
            .bind(500_000_i32)
            .bind(420_000_i32)
            .execute(&pool)
            .await?;

        let state: (
            String,
            String,
            Option<String>,
            Option<chrono::DateTime<chrono::Utc>>,
        ) = sqlx::query_as(
            "SELECT storage_state, index_state, transcribe_state, indexed_at
             FROM tracks WHERE sc_track_id = '42'",
        )
        .fetch_one(&pool)
        .await?;
        assert_eq!(
            state,
            (
                "too_long".to_owned(),
                "too_long".to_owned(),
                Some("disabled".to_owned()),
                None,
            )
        );
        Ok(())
    }

    #[sqlx::test(migrations = false)]
    async fn oversized_tracks_leave_no_hq_upgrade_work(pool: PgPool) -> anyhow::Result<()> {
        install_schema(&pool).await?;
        sqlx::query(
            "INSERT INTO tracks (
                 sc_track_id, duration_ms, needs_duration_resolve,
                 storage_state, storage_attempts, index_state, hq_upgrade_pending
             ) VALUES ('42', 30000, true, 'pending', 0, 'pending', true)",
        )
        .execute(&pool)
        .await?;

        sqlx::query(include_str!("../../../queries/indexing/duration/apply.sql"))
            .bind("42")
            .bind(500_000_i32)
            .bind(420_000_i32)
            .execute(&pool)
            .await?;

        let pending: bool =
            sqlx::query_scalar("SELECT hq_upgrade_pending FROM tracks WHERE sc_track_id = '42'")
                .fetch_one(&pool)
                .await?;
        assert!(!pending);
        Ok(())
    }

    #[sqlx::test(migrations = false)]
    async fn corrected_tracks_leave_the_too_long_terminal_state(
        pool: PgPool,
    ) -> anyhow::Result<()> {
        install_schema(&pool).await?;
        sqlx::query(
            "INSERT INTO tracks (
                 sc_track_id, duration_ms, needs_duration_resolve,
                 storage_state, storage_attempts, index_state, indexed_at, transcribe_state
             ) VALUES ('42', 500000, true, 'too_long', 3, 'too_long', now(), 'disabled')",
        )
        .execute(&pool)
        .await?;

        sqlx::query(include_str!("../../../queries/indexing/duration/apply.sql"))
            .bind("42")
            .bind(180_000_i32)
            .bind(420_000_i32)
            .execute(&pool)
            .await?;

        let state: (
            String,
            i16,
            String,
            Option<String>,
            Option<chrono::DateTime<chrono::Utc>>,
        ) = sqlx::query_as(
            "SELECT storage_state, storage_attempts, index_state, transcribe_state, indexed_at
             FROM tracks WHERE sc_track_id = '42'",
        )
        .fetch_one(&pool)
        .await?;
        assert_eq!(
            state,
            ("pending".to_owned(), 0, "pending".to_owned(), None, None)
        );
        Ok(())
    }

    #[sqlx::test(migrations = false)]
    async fn duration_and_index_job_commit_together(pool: PgPool) -> anyhow::Result<()> {
        install_schema(&pool).await?;
        sqlx::raw_sql(include_str!(
            "../../../../api/migrations/0057_background_jobs.sql"
        ))
        .execute(&pool)
        .await?;
        sqlx::query(
            "INSERT INTO tracks (
                 sc_track_id, duration_ms, needs_duration_resolve,
                 storage_state, storage_attempts, index_state
             ) VALUES ('42', 30000, true, 'pending', 0, 'pending')",
        )
        .execute(&pool)
        .await?;

        resolver(&pool)?
            .persist_duration("42".to_owned(), 180_000)
            .await?;

        let needs_resolve: bool = sqlx::query_scalar(
            "SELECT needs_duration_resolve FROM tracks WHERE sc_track_id = '42'",
        )
        .fetch_one(&pool)
        .await?;
        let queued: (String, Option<String>) = sqlx::query_as(
            "SELECT kind, dedup_key FROM background_jobs WHERE kind = 'indexing.track'",
        )
        .fetch_one(&pool)
        .await?;
        assert!(!needs_resolve);
        assert_eq!(queued, ("indexing.track".to_owned(), Some("42".to_owned())));
        Ok(())
    }

    #[sqlx::test(migrations = false)]
    async fn failed_index_enqueue_keeps_duration_pending(pool: PgPool) -> anyhow::Result<()> {
        install_schema(&pool).await?;
        sqlx::query(
            "INSERT INTO tracks (
                 sc_track_id, duration_ms, needs_duration_resolve,
                 storage_state, storage_attempts, index_state
             ) VALUES ('42', 30000, true, 'pending', 0, 'pending')",
        )
        .execute(&pool)
        .await?;

        assert!(
            resolver(&pool)?
                .persist_duration("42".to_owned(), 180_000)
                .await
                .is_err()
        );

        let state: (i32, bool) = sqlx::query_as(
            "SELECT duration_ms, needs_duration_resolve FROM tracks WHERE sc_track_id = '42'",
        )
        .fetch_one(&pool)
        .await?;
        assert_eq!(state, (30_000, true));
        Ok(())
    }
}
