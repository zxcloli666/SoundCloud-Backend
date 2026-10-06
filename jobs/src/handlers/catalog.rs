#[cfg(test)]
#[path = "catalog_tests.rs"]
mod tests;

use catalog_normalize::{NORMALIZER_VERSION, title_forms};
use sqlx::PgPool;
use tracing::info;
use uuid::Uuid;

use crate::queue::{JobError, JobResult};

const KEY_BATCH: i64 = 500;
const LINK_BATCH: i64 = 200;
const MERGE_GROUP_BATCH: i64 = 100;

pub struct CatalogWorkHandler {
    pool: PgPool,
}

struct TitleRow {
    id: Uuid,
    title: String,
}

#[derive(Default)]
struct KeyBatch {
    ids: Vec<Uuid>,
    work_keys: Vec<String>,
    recording_keys: Vec<String>,
    alias_ids: Vec<Uuid>,
    alias_keys: Vec<String>,
}

impl KeyBatch {
    fn build(rows: Vec<TitleRow>) -> Self {
        let mut batch = Self::default();
        for row in rows {
            let forms = title_forms(&row.title);
            batch.ids.push(row.id);
            batch.work_keys.push(forms.work_key);
            batch.recording_keys.push(forms.recording_key);
            for alias in forms.aliases {
                batch.alias_ids.push(row.id);
                batch.alias_keys.push(alias);
            }
        }
        batch
    }

    fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }
}

impl CatalogWorkHandler {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub async fn reconcile(&self) -> JobResult {
        let tracks = self.backfill_track_keys().await?;
        let wanted = self.backfill_wanted_keys().await?;
        if tracks > 0 || wanted > 0 {
            info!(tracks, wanted, "catalog work keys backfilled");
        }
        self.link_matching_works().await?;
        self.merge_duplicate_recordings().await
    }

    async fn merge_duplicate_recordings(&self) -> JobResult {
        let cursor = sqlx::query_file!("queries/catalog/load_merge_state.sql")
            .fetch_optional(&self.pool)
            .await
            .map_err(JobError::retryable)?
            .ok_or_else(|| {
                JobError::permanent(anyhow::anyhow!("catalog merge state is missing"))
            })?;

        let groups = sqlx::query_file!(
            "queries/catalog/next_duplicate_groups.sql",
            cursor.cursor_artist,
            cursor.cursor_recording_key,
            MERGE_GROUP_BATCH
        )
        .fetch_all(&self.pool)
        .await
        .map_err(JobError::retryable)?;

        let Some(last) = groups.last() else {
            sqlx::query_file!("queries/catalog/restart_merge_pass.sql")
                .execute(&self.pool)
                .await
                .map_err(JobError::retryable)?;
            return Ok(());
        };
        let next_cursor = (last.primary_artist_id, last.recording_key.clone());

        let mut merged = 0i64;
        let mut superseded = 0i64;
        for group in &groups {
            let outcome = sqlx::query_file!(
                "queries/catalog/merge_recording_group.sql",
                group.primary_artist_id,
                group.recording_key,
                Uuid::now_v7()
            )
            .fetch_one(&self.pool)
            .await
            .map_err(JobError::retryable)?;
            if outcome.superseded > 0 {
                merged += 1;
                superseded += outcome.superseded;
            }
        }

        sqlx::query_file!(
            "queries/catalog/advance_merge_cursor.sql",
            next_cursor.0,
            next_cursor.1,
            merged,
            superseded
        )
        .execute(&self.pool)
        .await
        .map_err(JobError::retryable)?;

        if merged > 0 {
            info!(
                groups = merged,
                superseded, "duplicate recordings merged behind a serving winner"
            );
        }
        Ok(())
    }

    async fn backfill_track_keys(&self) -> Result<usize, JobError> {
        let rows = sqlx::query_file_as!(
            TitleRow,
            "queries/catalog/claim_tracks_for_keys.sql",
            KEY_BATCH
        )
        .fetch_all(&self.pool)
        .await
        .map_err(JobError::retryable)?;

        let batch = KeyBatch::build(rows);
        if batch.is_empty() {
            return Ok(0);
        }

        let mut transaction = self.pool.begin().await.map_err(JobError::retryable)?;
        sqlx::query_file!(
            "queries/catalog/store_track_keys.sql",
            &batch.ids,
            &batch.work_keys,
            &batch.recording_keys,
            NORMALIZER_VERSION
        )
        .execute(&mut *transaction)
        .await
        .map_err(JobError::retryable)?;
        sqlx::query_file!("queries/catalog/clear_track_aliases.sql", &batch.ids)
            .execute(&mut *transaction)
            .await
            .map_err(JobError::retryable)?;
        sqlx::query_file!(
            "queries/catalog/store_track_aliases.sql",
            &batch.alias_ids,
            &batch.alias_keys
        )
        .execute(&mut *transaction)
        .await
        .map_err(JobError::retryable)?;
        transaction.commit().await.map_err(JobError::retryable)?;
        Ok(batch.ids.len())
    }

    async fn backfill_wanted_keys(&self) -> Result<usize, JobError> {
        let rows = sqlx::query_file_as!(
            TitleRow,
            "queries/catalog/claim_wanted_for_keys.sql",
            KEY_BATCH
        )
        .fetch_all(&self.pool)
        .await
        .map_err(JobError::retryable)?;

        let batch = KeyBatch::build(rows);
        if batch.is_empty() {
            return Ok(0);
        }

        let mut transaction = self.pool.begin().await.map_err(JobError::retryable)?;
        sqlx::query_file!(
            "queries/catalog/store_wanted_keys.sql",
            &batch.ids,
            &batch.work_keys,
            &batch.recording_keys,
            NORMALIZER_VERSION
        )
        .execute(&mut *transaction)
        .await
        .map_err(JobError::retryable)?;
        sqlx::query_file!("queries/catalog/clear_wanted_aliases.sql", &batch.ids)
            .execute(&mut *transaction)
            .await
            .map_err(JobError::retryable)?;
        sqlx::query_file!(
            "queries/catalog/store_wanted_aliases.sql",
            &batch.alias_ids,
            &batch.alias_keys
        )
        .execute(&mut *transaction)
        .await
        .map_err(JobError::retryable)?;
        transaction.commit().await.map_err(JobError::retryable)?;
        Ok(batch.ids.len())
    }

    async fn link_matching_works(&self) -> JobResult {
        let outcome = sqlx::query_file!(
            "queries/catalog/link_wanted_to_local.sql",
            LINK_BATCH,
            NORMALIZER_VERSION
        )
        .fetch_one(&self.pool)
        .await
        .map_err(JobError::retryable)?;

        if outcome.linked > 0 {
            info!(
                linked = outcome.linked,
                unmatched = outcome.unmatched,
                "external catalog entries linked to local tracks"
            );
        }
        Ok(())
    }
}
