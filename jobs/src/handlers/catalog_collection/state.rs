use backend_contracts::CatalogCollectionPayload;
use chrono::{DateTime, Utc};
use sqlx::{PgConnection, PgPool};
use uuid::Uuid;

use crate::queue::{JobError, JobResult, LeasedJob};

#[derive(Debug)]
pub(super) struct Snapshot {
    pub snapshot_id: Uuid,
    pub started_at: DateTime<Utc>,
    pub next_cursor: Option<String>,
    pub page_count: i64,
    pub item_count: i64,
    pub complete: bool,
}

pub(super) async fn fence(connection: &mut PgConnection, job: &LeasedJob) -> JobResult<bool> {
    Ok(sqlx::query_file_scalar!(
        "queries/catalog_refresh/fence.sql",
        job.id,
        job.lease_id,
        job.generation
    )
    .fetch_optional(connection)
    .await
    .map_err(JobError::retryable)?
    .is_some())
}

pub(super) async fn load(
    connection: &mut PgConnection,
    job: &LeasedJob,
    payload: &CatalogCollectionPayload,
) -> JobResult<Snapshot> {
    sqlx::query_file_as!(
        Snapshot,
        "queries/catalog_collection/load.sql",
        &payload.subject_id,
        payload.collection.as_str(),
        payload.scope(),
        job.id,
        job.generation
    )
    .fetch_one(connection)
    .await
    .map_err(JobError::retryable)
}

pub(super) async fn begin(
    pool: &PgPool,
    job: &LeasedJob,
    payload: &CatalogCollectionPayload,
) -> JobResult<Option<Snapshot>> {
    let mut tx = pool.begin().await.map_err(JobError::retryable)?;
    if !fence(&mut tx, job).await? {
        return Ok(None);
    }
    let proposed = Uuid::now_v7();
    sqlx::query_file!(
        "queries/catalog_collection/begin.sql",
        &payload.subject_id,
        payload.collection.as_str(),
        payload.scope(),
        job.id,
        job.generation,
        proposed
    )
    .execute(&mut *tx)
    .await
    .map_err(JobError::retryable)?;
    let snapshot = load(&mut tx, job, payload).await?;
    if snapshot.snapshot_id == proposed {
        sqlx::query_file!(
            "queries/catalog_collection/clear_cursors.sql",
            &payload.subject_id,
            payload.collection.as_str(),
            payload.scope()
        )
        .execute(&mut *tx)
        .await
        .map_err(JobError::retryable)?;
        sqlx::query_file!(
            "queries/catalog_collection/clear_seen.sql",
            &payload.subject_id,
            payload.collection.as_str(),
            payload.scope(),
            proposed
        )
        .execute(&mut *tx)
        .await
        .map_err(JobError::retryable)?;
    }
    tx.commit().await.map_err(JobError::retryable)?;
    Ok(Some(snapshot))
}

pub(super) async fn advance(
    connection: &mut PgConnection,
    payload: &CatalogCollectionPayload,
    snapshot: &Snapshot,
    keys: &[String],
    next: Option<&str>,
) -> JobResult {
    if let Some(cursor) = next {
        let inserted = sqlx::query_file!(
            "queries/catalog_collection/cursor.sql",
            &payload.subject_id,
            payload.collection.as_str(),
            payload.scope(),
            cursor
        )
        .execute(&mut *connection)
        .await
        .map_err(JobError::retryable)?;
        if inserted.rows_affected() == 0 {
            return Err(JobError::retryable(anyhow::anyhow!(
                "SoundCloud collection cursor cycle"
            )));
        }
    }
    sqlx::query_file!(
        "queries/catalog_collection/seen.sql",
        &payload.subject_id,
        payload.collection.as_str(),
        payload.scope(),
        snapshot.snapshot_id,
        keys
    )
    .execute(&mut *connection)
    .await
    .map_err(JobError::retryable)?;
    sqlx::query_file!(
        "queries/catalog_collection/advance.sql",
        &payload.subject_id,
        payload.collection.as_str(),
        payload.scope(),
        next,
        keys.len() as i64,
        next.is_none()
    )
    .execute(connection)
    .await
    .map_err(JobError::retryable)?;
    Ok(())
}
