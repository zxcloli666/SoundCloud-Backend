#[cfg(test)]
#[path = "search_terms_tests.rs"]
mod tests;

use std::time::Instant;

use sqlx::{Connection, PgConnection, PgPool};
use tracing::info;

use crate::queue::{JobError, JobResult};

const CHUNK_ROWS: i64 = 50_000;

pub struct SearchTermsHandler {
    pool: PgPool,
}

#[derive(Debug, Default, PartialEq, Eq)]
struct Refresh {
    changed: u64,
    chunks: u64,
    upserted: i64,
    removed: i64,
}

impl SearchTermsHandler {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub async fn refresh(&self) -> JobResult {
        let started = Instant::now();
        let mut connection = self.pool.acquire().await.map_err(JobError::retryable)?;
        connection.close_on_drop();
        let Some(refresh) = refresh(&mut connection, CHUNK_ROWS)
            .await
            .map_err(JobError::retryable)?
        else {
            info!("search terms refresh skipped because another refresh holds the lock");
            return Ok(());
        };
        info!(
            changed = refresh.changed,
            chunks = refresh.chunks,
            upserted = refresh.upserted,
            removed = refresh.removed,
            elapsed_ms = started.elapsed().as_millis() as u64,
            "search terms refreshed"
        );
        Ok(())
    }
}

async fn refresh(
    connection: &mut PgConnection,
    chunk_rows: i64,
) -> Result<Option<Refresh>, sqlx::Error> {
    let acquired = sqlx::query_file_scalar!("queries/search/lock_terms.sql")
        .fetch_one(&mut *connection)
        .await?;
    if !acquired {
        return Ok(None);
    }

    let mut transaction = connection.begin().await?;
    let changed = sqlx::query_file!("queries/search/build_terms_delta.sql", chunk_rows)
        .execute(&mut *transaction)
        .await?
        .rows_affected();
    sqlx::query(include_str!("../../queries/search/index_terms_delta.sql"))
        .execute(&mut *transaction)
        .await?;
    transaction.commit().await?;

    let mut refresh = Refresh {
        changed,
        chunks: changed.div_ceil(chunk_rows.unsigned_abs()),
        ..Refresh::default()
    };
    for chunk in 0..refresh.chunks {
        let (upserted, removed): (i64, i64) =
            sqlx::query_as(include_str!("../../queries/search/apply_terms_chunk.sql"))
                .bind(chunk as i64)
                .fetch_one(&mut *connection)
                .await?;
        refresh.upserted += upserted;
        refresh.removed += removed;
    }
    Ok(Some(refresh))
}
