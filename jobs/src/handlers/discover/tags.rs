use std::time::Instant;

use sqlx::{Connection, PgConnection, PgPool};
use tracing::{info, warn};

use super::paced::{run_statement, update_by_ranges};
use crate::queue::{JobError, JobResult};

const DROP_COUNTS: &str = include_str!("../../../queries/discover/artist_tags/drop_counts.sql");
const PREFER_SEQUENTIAL_SCAN: &str =
    include_str!("../../../queries/discover/artist_tags/prefer_sequential_scan.sql");
const CREATE_COUNTS: &str = include_str!("../../../queries/discover/artist_tags/create_counts.sql");
const INDEX_COUNTS: &str = include_str!("../../../queries/discover/artist_tags/index_counts.sql");
const ANALYZE_COUNTS: &str =
    include_str!("../../../queries/discover/artist_tags/analyze_counts.sql");
const UPDATE_RANGE: &str = include_str!("../../../queries/discover/artist_tags/update_range.sql");

pub(super) async fn refresh(pool: &PgPool) -> JobResult {
    let started = Instant::now();
    let mut connection = pool.acquire().await.map_err(JobError::retryable)?;
    let result = refresh_on(&mut connection).await;
    if let Err(error) = run_statement(&mut connection, DROP_COUNTS).await {
        warn!(%error, "artist genre counts were not dropped");
    }
    let updated = result?;
    info!(
        updated,
        elapsed_ms = started.elapsed().as_millis() as u64,
        "artist tags refreshed"
    );
    Ok(())
}

async fn refresh_on(connection: &mut PgConnection) -> Result<u64, JobError> {
    run_statement(connection, DROP_COUNTS).await?;
    count_genres(connection).await?;
    run_statement(connection, INDEX_COUNTS).await?;
    run_statement(connection, ANALYZE_COUNTS).await?;
    update_by_ranges(connection, UPDATE_RANGE).await
}

async fn count_genres(connection: &mut PgConnection) -> JobResult {
    let mut transaction = connection.begin().await.map_err(JobError::retryable)?;
    run_statement(&mut transaction, PREFER_SEQUENTIAL_SCAN).await?;
    run_statement(&mut transaction, CREATE_COUNTS).await?;
    transaction.commit().await.map_err(JobError::retryable)?;
    Ok(())
}
