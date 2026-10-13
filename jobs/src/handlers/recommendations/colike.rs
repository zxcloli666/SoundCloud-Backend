use sqlx::PgPool;

use crate::queue::{JobError, JobResult};

const SHRINKAGE: f64 = 8.0;
const TOP_K_PER_ARTIST: i64 = 80;
const REBUILD_SESSION: [&str; 2] = [
    "SET LOCAL statement_timeout = '7000s'",
    "SET LOCAL work_mem = '512MB'",
];

pub(super) async fn rebuild(pool: &PgPool) -> JobResult<u64> {
    let mut transaction = pool.begin().await.map_err(JobError::retryable)?;
    for setting in REBUILD_SESSION {
        sqlx::query(setting)
            .execute(&mut *transaction)
            .await
            .map_err(JobError::retryable)?;
    }
    let rebuilt = sqlx::query_file!(
        "queries/recommendations/colike/rebuild.sql",
        SHRINKAGE,
        TOP_K_PER_ARTIST
    )
    .execute(&mut *transaction)
    .await
    .map_err(JobError::retryable)?;
    transaction.commit().await.map_err(JobError::retryable)?;
    Ok(rebuilt.rows_affected())
}
