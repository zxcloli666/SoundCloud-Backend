#[cfg(test)]
#[path = "colike_tests.rs"]
mod tests;

use std::time::Duration;

use chrono::{DateTime, Utc};
use sqlx::{Connection, PgConnection, PgPool, Postgres, Transaction};
use uuid::Uuid;

use crate::queue::{JobError, JobResult};

const SHRINKAGE: f64 = 8.0;
const TOP_K_PER_ARTIST: i64 = 80;
const WEIGHT_TOLERANCE: f64 = 0.01;
const BUILD_SESSION: [&str; 2] = [
    "SET LOCAL statement_timeout = '1800s'",
    "SET LOCAL work_mem = '512MB'",
];
const PREPARE_FRESH: [&str; 4] = [
    include_str!("../../../queries/recommendations/colike/likers_fresh.sql"),
    include_str!("../../../queries/recommendations/colike/likers_key.sql"),
    include_str!("../../../queries/recommendations/colike/fresh_table.sql"),
    include_str!("../../../queries/recommendations/colike/analyze_users.sql"),
];
const PAIRS_BY_A: &str = include_str!("../../../queries/recommendations/colike/pairs_by_a.sql");
const PAIRS_BY_B: &str = include_str!("../../../queries/recommendations/colike/pairs_by_b.sql");
const PACING: Pacing = Pacing {
    shards: 64,
    batch_edges: 500,
    pause: Duration::from_millis(150),
};

#[derive(Clone, Copy)]
struct Pacing {
    shards: i32,
    batch_edges: i64,
    pause: Duration,
}

#[derive(Debug, Default)]
pub(super) struct Rebuild {
    pub(super) fresh: u64,
    pub(super) batches: u64,
    pub(super) upserted: i64,
    pub(super) removed: i64,
}

pub(super) async fn rebuild(pool: &PgPool) -> JobResult<Rebuild> {
    let mut connection = pool.acquire().await.map_err(JobError::retryable)?;
    connection.close_on_drop();
    rebuild_on(&mut connection, PACING)
        .await
        .map_err(JobError::retryable)
}

async fn rebuild_on(connection: &mut PgConnection, pacing: Pacing) -> Result<Rebuild, sqlx::Error> {
    let (fresh, started_at) = build_fresh(connection, pacing).await?;
    let mut rebuild = Rebuild {
        fresh,
        ..Rebuild::default()
    };
    let mut lower = Uuid::nil();
    loop {
        let Some(upper) = batch_upper(connection, lower, pacing.batch_edges).await? else {
            return Ok(rebuild);
        };
        let (upserted, removed) = apply_batch(connection, lower, upper, started_at).await?;
        rebuild.batches += 1;
        rebuild.upserted += upserted;
        rebuild.removed += removed;
        let Some(next) = upper.as_u128().checked_add(1) else {
            return Ok(rebuild);
        };
        lower = Uuid::from_u128(next);
        tokio::time::sleep(pacing.pause).await;
    }
}

async fn build_fresh(
    connection: &mut PgConnection,
    pacing: Pacing,
) -> Result<(u64, DateTime<Utc>), sqlx::Error> {
    let mut transaction = build_step(connection).await?;
    let started_at = sqlx::query_file_scalar!("queries/recommendations/colike/started_at.sql")
        .fetch_one(&mut *transaction)
        .await?;
    sqlx::query_file!(
        "queries/recommendations/colike/users_fresh.sql",
        pacing.shards
    )
    .execute(&mut *transaction)
    .await?;
    for statement in PREPARE_FRESH {
        sqlx::query(statement).execute(&mut *transaction).await?;
    }
    transaction.commit().await?;
    let mut fresh = 0;
    for pairs in [PAIRS_BY_A, PAIRS_BY_B] {
        for shard in 0..pacing.shards {
            let mut transaction = build_step(connection).await?;
            fresh += sqlx::query(pairs)
                .bind(SHRINKAGE)
                .bind(TOP_K_PER_ARTIST)
                .bind(shard)
                .execute(&mut *transaction)
                .await?
                .rows_affected();
            transaction.commit().await?;
            tokio::time::sleep(pacing.pause).await;
        }
    }
    sqlx::query(include_str!(
        "../../../queries/recommendations/colike/analyze_fresh.sql"
    ))
    .execute(&mut *connection)
    .await?;
    Ok((fresh, started_at))
}

async fn build_step(
    connection: &mut PgConnection,
) -> Result<Transaction<'_, Postgres>, sqlx::Error> {
    let mut transaction = connection.begin().await?;
    for setting in BUILD_SESSION {
        sqlx::query(setting).execute(&mut *transaction).await?;
    }
    Ok(transaction)
}

async fn batch_upper(
    connection: &mut PgConnection,
    lower: Uuid,
    batch_edges: i64,
) -> Result<Option<Uuid>, sqlx::Error> {
    sqlx::query_scalar(include_str!(
        "../../../queries/recommendations/colike/batch_upper.sql"
    ))
    .bind(lower)
    .bind(batch_edges)
    .fetch_one(connection)
    .await
}

async fn apply_batch(
    connection: &mut PgConnection,
    lower: Uuid,
    upper: Uuid,
    started_at: DateTime<Utc>,
) -> Result<(i64, i64), sqlx::Error> {
    sqlx::query_as(include_str!(
        "../../../queries/recommendations/colike/apply_batch.sql"
    ))
    .bind(lower)
    .bind(upper)
    .bind(started_at)
    .bind(WEIGHT_TOLERANCE)
    .fetch_one(connection)
    .await
}
