use sqlx::{Executor, PgPool};

use crate::queue::{JobError, JobResult};

const CLEANUP_BATCH: i64 = 1_000;

pub struct MaintenanceHandler {
    pool: PgPool,
}

impl MaintenanceHandler {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
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
        let mut transaction = self.pool.begin().await.map_err(JobError::retryable)?;
        sqlx::query_file!("queries/maintenance/cleanup_job_failures.sql")
            .execute(&mut *transaction)
            .await
            .map_err(JobError::retryable)?;
        sqlx::query_file!("queries/maintenance/cleanup_job_receipts.sql")
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

    pub async fn refresh_search_terms(&self) -> JobResult {
        let mut transaction = self.pool.begin().await.map_err(JobError::retryable)?;
        transaction
            .execute(sqlx::raw_sql(include_str!(
                "../../queries/search/refresh_terms.sql"
            )))
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

    #[sqlx::test(migrations = "../api/migrations")]
    async fn the_search_lexicon_learns_catalog_words_but_not_whisper_gibberish(
        pool: PgPool,
    ) -> anyhow::Result<()> {
        sqlx::raw_sql(
            "INSERT INTO tracks (sc_track_id, urn, title, title_normalized, sharing, duration_ms, uploader_username)
             VALUES ('1', 'soundcloud:tracks:1', 'Umbrella', 'umbrella', 'public', 1000, 'rihanna'),
                    ('2', 'soundcloud:tracks:2', 'Umbrella', 'umbrella', 'public', 1000, 'rihanna'),
                    ('3', 'soundcloud:tracks:3', 'Hidden', 'hidden', 'private', 1000, 'rihanna');
             INSERT INTO lyrics_cache (sc_track_id, plain_text, source)
             VALUES ('1', 'zzgibber', 'self_gen'), ('2', 'zzgibber', 'self_gen')",
        )
        .execute(&pool)
        .await?;

        super::MaintenanceHandler::new(pool.clone())
            .refresh_search_terms()
            .await
            .map_err(|error| anyhow::anyhow!(error.to_string()))?;

        let words: Vec<(String, i32)> =
            sqlx::query_as("SELECT word, ndoc FROM search_terms ORDER BY word")
                .fetch_all(&pool)
                .await?;
        assert_eq!(
            words,
            vec![("rihanna".to_owned(), 2), ("umbrella".to_owned(), 2)]
        );
        Ok(())
    }
}
