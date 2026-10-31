use backend_contracts::{JobKind, LyricsLookupPayload, Versioned};
use chrono::Utc;
use sqlx::{PgExecutor, PgPool, Postgres, Transaction};

use crate::queue::{JobError, JobRepository, JobResult, NewJob, QueueError};

const PRIORITY: i16 = 10;
const MAX_ATTEMPTS: i16 = 8;

struct Wakes {
    sc_track_ids: Vec<String>,
    payloads: Vec<serde_json::Value>,
}

impl Wakes {
    fn of(sc_track_ids: &[String]) -> JobResult<Self> {
        let mut sorted = sc_track_ids.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        let payloads = sorted
            .iter()
            .map(|sc_track_id| {
                serde_json::to_value(Versioned::V1(LyricsLookupPayload {
                    sc_track_id: sc_track_id.clone(),
                }))
                .map_err(JobError::permanent)
            })
            .collect::<JobResult<Vec<_>>>()?;
        Ok(Self {
            sc_track_ids: sorted,
            payloads,
        })
    }
}

pub async fn enqueue(pool: &PgPool, queue: &JobRepository, sc_track_id: &str) -> JobResult {
    enqueue_many(pool, queue, &[sc_track_id.to_owned()]).await
}

pub async fn enqueue_many(
    pool: &PgPool,
    queue: &JobRepository,
    sc_track_ids: &[String],
) -> JobResult {
    if sc_track_ids.is_empty() {
        return Ok(());
    }
    let batch = match Wakes::of(sc_track_ids) {
        Ok(wakes) => deliver(pool, &wakes).await,
        Err(error) => Err(error),
    };
    let Err(error) = batch else {
        return Ok(());
    };
    tracing::debug!(%error, "lyrics wakes fall back to one transaction per track");
    enqueue_one_by_one(pool, queue, sc_track_ids).await
}

pub async fn enqueue_all_in(
    transaction: &mut Transaction<'_, Postgres>,
    sc_track_ids: &[String],
) -> JobResult {
    if sc_track_ids.is_empty() {
        return Ok(());
    }
    deliver(&mut **transaction, &Wakes::of(sc_track_ids)?).await
}

async fn deliver<'e>(executor: impl PgExecutor<'e>, wakes: &Wakes) -> JobResult {
    let kind = JobKind::LyricsLookup;
    sqlx::query_file!(
        "queries/lyrics/lookup_deliver_wakes.sql",
        &wakes.sc_track_ids,
        &wakes.payloads,
        kind.as_str(),
        kind.lane().as_str(),
        PRIORITY,
        MAX_ATTEMPTS,
        Utc::now()
    )
    .execute(executor)
    .await
    .map_err(JobError::retryable)?;
    Ok(())
}

async fn enqueue_one_by_one(
    pool: &PgPool,
    queue: &JobRepository,
    sc_track_ids: &[String],
) -> JobResult {
    let pending = sqlx::query_file_scalar!("queries/lyrics/lookup_pending_wakes.sql", sc_track_ids)
        .fetch_all(pool)
        .await
        .map_err(JobError::retryable)?;
    let mut first_error = None;
    for sc_track_id in pending {
        if let Err(error) = enqueue_pending(pool, queue, &sc_track_id).await {
            first_error.get_or_insert(error);
        }
    }
    first_error.map_or(Ok(()), Err)
}

async fn enqueue_pending(pool: &PgPool, queue: &JobRepository, sc_track_id: &str) -> JobResult {
    let mut transaction = pool.begin().await.map_err(JobError::retryable)?;
    enqueue_in(queue, &mut transaction, sc_track_id).await?;
    transaction.commit().await.map_err(JobError::retryable)
}

pub(super) async fn enqueue_in(
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
