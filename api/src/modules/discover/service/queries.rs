use std::time::Duration;

use crate::error::{AppError, AppResult};

use super::cache_runtime::with_timeout;
use super::{CachedSummary, CachedTag, CachedTagList, DiscoverService};

const DATABASE_ACQUIRE_TIMEOUT: Duration = Duration::from_secs(5);
const STATEMENT_TIMEOUT_MS: u32 = 15_000;
const LOCK_TIMEOUT_MS: u32 = 500;
const FRESH_WINDOW_DAYS: i32 = 14;
const TAG_PRECOMPUTE_LIMIT: i64 = 32;

impl DiscoverService {
    pub(super) async fn compute_summary(&self) -> AppResult<CachedSummary> {
        let mut transaction = self.bounded_cache_read().await?;
        let row = sqlx::query_file!(
            "queries/discover/service/compute_summary.sql",
            FRESH_WINDOW_DAYS
        )
        .fetch_one(&mut *transaction)
        .await?;
        transaction.commit().await?;

        Ok(CachedSummary {
            artists_count: row.artists_count.max(0),
            albums_count: row.albums_count.max(0),
            fresh_count: row.fresh_count,
            fresh_window_days: FRESH_WINDOW_DAYS,
        })
    }

    pub(super) async fn compute_tag_list(&self) -> AppResult<CachedTagList> {
        let mut transaction = self.bounded_cache_read().await?;
        let rows = sqlx::query_file!(
            "queries/discover/service/compute_tag_list.sql",
            TAG_PRECOMPUTE_LIMIT
        )
        .fetch_all(&mut *transaction)
        .await?;
        transaction.commit().await?;

        Ok(CachedTagList {
            items: rows
                .into_iter()
                .map(|row| CachedTag {
                    id: row.tag,
                    count: row.n,
                })
                .collect(),
        })
    }

    async fn bounded_cache_read(&self) -> AppResult<sqlx::Transaction<'_, sqlx::Postgres>> {
        let mut transaction = with_timeout(
            DATABASE_ACQUIRE_TIMEOUT,
            "discover cache database acquisition timed out",
            async { self.pg.begin().await.map_err(AppError::from) },
        )
        .await?;
        sqlx::query(&format!(
            "SET LOCAL statement_timeout = {STATEMENT_TIMEOUT_MS}"
        ))
        .execute(&mut *transaction)
        .await?;
        sqlx::query(&format!("SET LOCAL lock_timeout = {LOCK_TIMEOUT_MS}"))
            .execute(&mut *transaction)
            .await?;
        Ok(transaction)
    }
}
