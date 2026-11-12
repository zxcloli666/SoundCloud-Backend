use std::time::Instant;

use sqlx::PgPool;
use tracing::{info, warn};

use super::paced::{run_statement, update_by_ranges};
use crate::queue::{JobError, JobResult};

const INTERNAL_PLAY_WEIGHT: i64 = 10_000;
const DROP_TOTALS: &str =
    include_str!("../../../queries/discover/artist_popularity/drop_totals.sql");
const CREATE_TOTALS: &str =
    include_str!("../../../queries/discover/artist_popularity/create_totals.sql");
const INDEX_TOTALS: &str =
    include_str!("../../../queries/discover/artist_popularity/index_totals.sql");
const ANALYZE_TOTALS: &str =
    include_str!("../../../queries/discover/artist_popularity/analyze_totals.sql");
const CREATE_MAXIMUM: &str =
    include_str!("../../../queries/discover/artist_popularity/create_maximum.sql");
const UPDATE_RANGE: &str =
    include_str!("../../../queries/discover/artist_popularity/update_range.sql");

pub(super) async fn refresh(pool: &PgPool) -> JobResult {
    let started = Instant::now();
    let mut connection = pool.acquire().await.map_err(JobError::retryable)?;
    let result = refresh_on(&mut connection).await;
    if let Err(error) = run_statement(&mut connection, DROP_TOTALS).await {
        warn!(%error, "artist play totals were not dropped");
    }
    let updated = result?;
    info!(
        updated,
        elapsed_ms = started.elapsed().as_millis() as u64,
        "artist popularity refreshed"
    );
    Ok(())
}

async fn refresh_on(connection: &mut sqlx::PgConnection) -> Result<u64, JobError> {
    run_statement(connection, DROP_TOTALS).await?;
    sqlx::query(CREATE_TOTALS)
        .bind(INTERNAL_PLAY_WEIGHT)
        .execute(&mut *connection)
        .await
        .map_err(JobError::retryable)?;
    run_statement(connection, INDEX_TOTALS).await?;
    run_statement(connection, ANALYZE_TOTALS).await?;
    run_statement(connection, CREATE_MAXIMUM).await?;
    update_by_ranges(connection, UPDATE_RANGE).await
}
