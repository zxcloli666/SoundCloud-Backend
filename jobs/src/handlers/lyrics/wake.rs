use backend_contracts::{JobKind, LyricsLookupPayload, Versioned};
use chrono::Utc;
use sqlx::{PgPool, Postgres, Transaction};

use crate::queue::{JobError, JobRepository, JobResult, NewJob, QueueError};

const PRIORITY: i16 = 10;
const MAX_ATTEMPTS: i16 = 8;

pub async fn enqueue(pool: &PgPool, queue: &JobRepository, sc_track_id: &str) -> JobResult {
    let mut transaction = pool.begin().await.map_err(JobError::retryable)?;
    enqueue_in(queue, &mut transaction, sc_track_id).await?;
    transaction.commit().await.map_err(JobError::retryable)
}

pub async fn enqueue_in(
    queue: &JobRepository,
    transaction: &mut Transaction<'_, Postgres>,
    sc_track_id: &str,
) -> JobResult {
    let wake = sqlx::query_file!("queries/lyrics/lookup_wake.sql", sc_track_id)
        .fetch_optional(&mut **transaction)
        .await
        .map_err(JobError::retryable)?;
    let Some(wake) = wake else {
        return Ok(());
    };
    let payload = LyricsLookupPayload {
        sc_track_id: sc_track_id.to_owned(),
    };
    let job = NewJob {
        id: wake.wake_message_id,
        kind: JobKind::LyricsLookup,
        dedup_key: Some(sc_track_id.to_owned()),
        payload: serde_json::to_value(Versioned::V1(payload)).map_err(JobError::permanent)?,
        priority: PRIORITY,
        max_attempts: MAX_ATTEMPTS,
        available_at: Utc::now(),
    };
    queue
        .enqueue_in_if_absent(transaction, &job)
        .await
        .map_err(queue_error)?;
    sqlx::query_file!(
        "queries/lyrics/lookup_mark_wake_durable.sql",
        wake.track_id,
        wake.generation,
        wake.wake_message_id
    )
    .execute(&mut **transaction)
    .await
    .map_err(JobError::retryable)?;
    Ok(())
}

fn queue_error(error: QueueError) -> JobError {
    match error {
        QueueError::Database(error) => JobError::retryable(error),
        error => JobError::permanent(error),
    }
}
