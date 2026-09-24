#[cfg(test)]
#[path = "reaper_tests.rs"]
mod tests;

use backend_contracts::worker_contract::{LYRICS_LANE, TRANSCRIBE_LANE, WorkerLaneSpec};
use backend_contracts::{JobKind, StoredAudioDispatchPayload, Versioned};
use chrono::Utc;
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

use crate::config::WorkerDispatchConfig;
use crate::queue::{JobError, JobRepository, JobResult, NewJob, QueueError};

use super::embedding_queue;

const ALIGN_BATCH: i64 = 30;
const REOPEN_BATCH: i64 = 30;
const EMBEDDING_BATCH: i64 = 500;
const QUARANTINE_BATCH: i64 = 50;
const TRANSCRIPTION_PRIORITY: i16 = 10;
const MAX_ATTEMPTS: i16 = 8;
const REOPEN_COOLDOWN_SECONDS: i64 = 6 * 60 * 60;
const MAX_TRANSCRIPTION_REOPENS: i32 = 7;

pub struct LyricsReaper {
    pool: PgPool,
    queue: JobRepository,
    dispatch: WorkerDispatchConfig,
}

struct TranscriptionCandidate {
    sc_track_id: String,
    uploaded_generation: i64,
}

impl LyricsReaper {
    pub fn new(pool: PgPool, dispatch: WorkerDispatchConfig) -> Self {
        Self {
            queue: JobRepository::new(pool.clone(), "lyrics-reaper".to_owned()),
            pool,
            dispatch,
        }
    }

    pub async fn reap_transcriptions(&self) -> JobResult {
        let result_window = result_window(&TRANSCRIBE_LANE)?;
        let retry_days = i64::try_from(self.dispatch.lyrics_align_rejected_retry_days)
            .map_err(|_| JobError::permanent(anyhow::anyhow!("rejected retry days overflow")))?;
        let mut transaction = self.pool.begin().await.map_err(JobError::retryable)?;
        let stale = sqlx::query_file_scalar!(
            "queries/lyrics/quarantine_stale_transcriptions.sql",
            QUARANTINE_BATCH,
            result_window
        )
        .fetch_one(&mut *transaction)
        .await
        .map_err(JobError::retryable)?;
        let orphaned = sqlx::query_file_scalar!(
            "queries/lyrics/quarantine_orphaned_transcriptions.sql",
            QUARANTINE_BATCH,
            result_window
        )
        .fetch_one(&mut *transaction)
        .await
        .map_err(JobError::retryable)?;
        let unreopenable = sqlx::query_file_scalar!(
            "queries/lyrics/settle_unreopenable_transcriptions.sql",
            REOPEN_COOLDOWN_SECONDS,
            MAX_TRANSCRIPTION_REOPENS,
            QUARANTINE_BATCH
        )
        .fetch_one(&mut *transaction)
        .await
        .map_err(JobError::retryable)?;

        let mut reopened = 0usize;
        let mut enqueued = 0usize;
        if self.dispatch.transcribe {
            let reopen = sqlx::query_file_as!(
                TranscriptionCandidate,
                "queries/lyrics/reopen_transcriptions.sql",
                REOPEN_COOLDOWN_SECONDS,
                MAX_TRANSCRIPTION_REOPENS,
                retry_days,
                REOPEN_BATCH
            )
            .fetch_all(&mut *transaction)
            .await
            .map_err(JobError::retryable)?;
            for candidate in reopen {
                enqueue_transcription(&self.queue, &mut transaction, candidate).await?;
                reopened += 1;
            }
            let align = sqlx::query_file_as!(
                TranscriptionCandidate,
                "queries/lyrics/reap_transcriptions_align.sql",
                ALIGN_BATCH
            )
            .fetch_all(&mut *transaction)
            .await
            .map_err(JobError::retryable)?;
            for candidate in align {
                enqueue_transcription(&self.queue, &mut transaction, candidate).await?;
                enqueued += 1;
            }
        }
        transaction.commit().await.map_err(JobError::retryable)?;
        if enqueued > 0 || reopened > 0 {
            tracing::info!(enqueued, reopened, "lyrics transcription jobs enqueued");
        }
        let quarantined = stale + orphaned + unreopenable;
        if quarantined > 0 {
            tracing::warn!(quarantined, "transcription attempts quarantined");
        }
        Ok(())
    }

    pub async fn reap_embeddings(&self) -> JobResult {
        let result_window = result_window(&LYRICS_LANE)?;
        let mut transaction = self.pool.begin().await.map_err(JobError::retryable)?;
        let stale = sqlx::query_file_scalar!(
            "queries/lyrics/quarantine_stale_embeddings.sql",
            QUARANTINE_BATCH,
            result_window
        )
        .fetch_one(&mut *transaction)
        .await
        .map_err(JobError::retryable)?;
        let orphaned_requests = sqlx::query_file_scalar!(
            "queries/lyrics/quarantine_orphaned_embedding_requests.sql",
            QUARANTINE_BATCH,
            result_window
        )
        .fetch_one(&mut *transaction)
        .await
        .map_err(JobError::retryable)?;
        let orphaned_lyrics = sqlx::query_file_scalar!(
            "queries/lyrics/quarantine_orphaned_embeddings.sql",
            QUARANTINE_BATCH
        )
        .fetch_one(&mut *transaction)
        .await
        .map_err(JobError::retryable)?;

        let mut enqueued = 0usize;
        if self.dispatch.embed_lyrics {
            let candidates =
                sqlx::query_file_scalar!("queries/lyrics/reap_embeddings.sql", EMBEDDING_BATCH)
                    .fetch_all(&mut *transaction)
                    .await
                    .map_err(JobError::retryable)?;
            for sc_track_id in candidates {
                if embedding_queue::enqueue_if_new(&self.queue, &mut transaction, &sc_track_id)
                    .await?
                {
                    enqueued += 1;
                }
            }
        }
        transaction.commit().await.map_err(JobError::retryable)?;
        if enqueued > 0 {
            tracing::info!(enqueued, "lyrics embedding jobs enqueued");
        }
        let quarantined = stale + orphaned_requests + orphaned_lyrics;
        if quarantined > 0 {
            tracing::warn!(quarantined, "stale lyrics embeddings quarantined");
        }
        Ok(())
    }
}

fn result_window(lane: &WorkerLaneSpec) -> JobResult<i64> {
    lane.quarantine_after_s()
        .and_then(|seconds| i64::try_from(seconds).ok())
        .ok_or_else(|| {
            JobError::permanent(anyhow::anyhow!(
                "worker lane {} has no result window",
                lane.lane.as_str()
            ))
        })
}

async fn enqueue_transcription(
    queue: &JobRepository,
    transaction: &mut Transaction<'_, Postgres>,
    candidate: TranscriptionCandidate,
) -> JobResult {
    let job = NewJob {
        id: Uuid::now_v7(),
        kind: JobKind::DispatchTranscription,
        dedup_key: Some(candidate.sc_track_id.clone()),
        payload: serde_json::to_value(Versioned::V1(StoredAudioDispatchPayload {
            sc_track_id: candidate.sc_track_id,
            uploaded_generation: candidate.uploaded_generation,
        }))
        .map_err(JobError::permanent)?,
        priority: TRANSCRIPTION_PRIORITY,
        max_attempts: MAX_ATTEMPTS,
        available_at: Utc::now(),
    };
    queue
        .enqueue_in_if_absent(transaction, &job)
        .await
        .map_err(queue_error)
}

fn queue_error(error: QueueError) -> JobError {
    match error {
        QueueError::Database(error) => JobError::retryable(error),
        error => JobError::permanent(error),
    }
}
