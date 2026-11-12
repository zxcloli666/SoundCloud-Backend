use std::time::Duration;

use sqlx::{Connection, PgConnection, Postgres, Transaction};
use uuid::Uuid;

use crate::queue::{JobError, JobResult};

const RANGES: u128 = 256;
const RANGE_BITS: u32 = 120;
const WRITE_ROWS_PER_SECOND: u64 = 150;
const LOCK_ATTEMPTS: u32 = 60;
const LOCK_RETRY: Duration = Duration::from_secs(2);
const LOCK_GIVE_UP_RETRY: Duration = Duration::from_secs(60);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct IdRange {
    pub(super) first: Uuid,
    pub(super) last: Uuid,
}

pub(super) fn id_ranges() -> impl Iterator<Item = IdRange> {
    (0..RANGES).map(|index| {
        let first = index << RANGE_BITS;
        let last = first | ((1u128 << RANGE_BITS) - 1);
        IdRange {
            first: Uuid::from_u128(first),
            last: Uuid::from_u128(last),
        }
    })
}

pub(super) async fn update_by_ranges(
    connection: &mut PgConnection,
    statement: &'static str,
) -> Result<u64, JobError> {
    let mut updated = 0;
    for range in id_ranges() {
        let mut transaction = begin_locked(connection).await?;
        let rows = sqlx::query(statement)
            .bind(range.first)
            .bind(range.last)
            .execute(&mut *transaction)
            .await
            .map_err(JobError::retryable)?
            .rows_affected();
        transaction.commit().await.map_err(JobError::retryable)?;
        updated += rows;
        pause_after(rows).await;
    }
    Ok(updated)
}

pub(super) async fn run_statement(
    connection: &mut PgConnection,
    statement: &'static str,
) -> JobResult {
    sqlx::query(statement)
        .execute(connection)
        .await
        .map_err(JobError::retryable)?;
    Ok(())
}

async fn begin_locked(
    connection: &mut PgConnection,
) -> Result<Transaction<'_, Postgres>, JobError> {
    let mut transaction = connection.begin().await.map_err(JobError::retryable)?;
    for _ in 0..LOCK_ATTEMPTS {
        let acquired = sqlx::query_file_scalar!("queries/discover/try_artist_write_lock.sql")
            .fetch_one(&mut *transaction)
            .await
            .map_err(JobError::retryable)?;
        if acquired {
            return Ok(transaction);
        }
        tokio::time::sleep(LOCK_RETRY).await;
    }
    Err(JobError::postponed(
        LOCK_GIVE_UP_RETRY,
        anyhow::anyhow!("another discover artist update is still running"),
    ))
}

async fn pause_after(rows: u64) {
    if rows > 0 {
        tokio::time::sleep(Duration::from_millis(rows * 1000 / WRITE_ROWS_PER_SECOND)).await;
    }
}
