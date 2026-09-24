use backend_contracts::{JobKind, LyricsEmbedPayload, Versioned};
use chrono::Utc;
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

use crate::queue::{JobError, JobRepository, JobResult, NewJob, QueueError};

const PRIORITY: i16 = 10;
const MAX_ATTEMPTS: i16 = 8;

pub async fn enqueue_if_new(
    queue: &JobRepository,
    transaction: &mut Transaction<'_, Postgres>,
    sc_track_id: &str,
) -> JobResult<bool> {
    let inserted = sqlx::query_file_scalar!("queries/lyrics/queue_embedding.sql", sc_track_id)
        .fetch_optional(&mut **transaction)
        .await
        .map_err(JobError::retryable)?;
    if inserted.is_none() {
        return Ok(false);
    }

    let payload = LyricsEmbedPayload {
        sc_track_id: sc_track_id.to_owned(),
    };
    let job = NewJob {
        id: Uuid::now_v7(),
        kind: JobKind::LyricsEmbed,
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
    Ok(true)
}

fn queue_error(error: QueueError) -> JobError {
    match error {
        QueueError::Database(error) => JobError::retryable(error),
        error => JobError::permanent(error),
    }
}
