use backend_contracts::{JobKind, LyricsLookupPayload, Versioned};
use chrono::Utc;
use sqlx::postgres::PgListener;
use sqlx::{PgExecutor, PgPool, Postgres, Transaction};
use std::time::Duration;
use tokio_util::sync::CancellationToken;

use crate::queue::{JobError, JobRepository, JobResult, NewJob, QueueError};

const PRIORITY: i16 = 10;
const MAX_ATTEMPTS: i16 = 8;
const CHANNEL: &str = "lyrics_wake";
const RECONNECT_DELAY: Duration = Duration::from_secs(5);

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

pub async fn listen(pool: PgPool, queue: JobRepository, cancellation: CancellationToken) {
    while !cancellation.is_cancelled() {
        if let Err(error) = listen_once(&pool, &queue, &cancellation).await {
            tracing::warn!(%error, "lyrics wake listener lost its connection");
        }
        tokio::select! {
            _ = cancellation.cancelled() => {}
            _ = tokio::time::sleep(RECONNECT_DELAY) => {}
        }
    }
}

async fn listen_once(
    pool: &PgPool,
    queue: &JobRepository,
    cancellation: &CancellationToken,
) -> Result<(), sqlx::Error> {
    let mut listener = PgListener::connect_with(pool).await?;
    listener.listen(CHANNEL).await?;
    loop {
        let notification = tokio::select! {
            _ = cancellation.cancelled() => return Ok(()),
            notification = listener.recv() => notification?,
        };
        if let Err(error) = enqueue(pool, queue, notification.payload()).await {
            tracing::warn!(%error, "lyrics wake from notify failed, the sweep will retry it");
        }
    }
}

pub async fn enqueue_due(pool: &PgPool, queue: &JobRepository, limit: i64) -> JobResult {
    let due = sqlx::query_file_scalar!("queries/lyrics/lookup_due_wakes.sql", limit)
        .fetch_all(pool)
        .await
        .map_err(JobError::retryable)?;
    enqueue_many(pool, queue, &due).await
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
