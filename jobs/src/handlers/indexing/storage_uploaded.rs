#[cfg(test)]
#[path = "storage_uploaded_tests.rs"]
mod tests;

use anyhow::ensure;
use backend_contracts::pipeline::{AudioIndexRequest, INDEX_AUDIO, StorageTrackUploaded};
use backend_contracts::{JobKind, StoredAudioDispatchPayload, Versioned};
use chrono::Utc;
use sqlx::PgPool;
use url::Url;
use uuid::Uuid;

use crate::bus::{Bus, DeliveryContext};
use crate::queue::{JobError, JobRepository, JobResult, NewJob, QueueError};

const MAX_ATTEMPTS: i16 = 8;
const DISPATCH_PRIORITY: i16 = 10;
const MAX_STORAGE_URL_BYTES: usize = 4_096;

pub struct StorageUploadHandler {
    pool: PgPool,
    queue: JobRepository,
    bus: Bus,
    storage_url: Url,
    max_track_duration_ms: i32,
}

struct UploadedAudio {
    sc_track_id: String,
    quality: Option<&'static str>,
}

#[derive(Debug)]
struct AppliedUpload {
    sc_track_id: String,
    generation: i64,
    too_long: bool,
    applied: bool,
}

#[derive(Debug)]
struct AudioDispatch {
    sc_track_id: String,
    upload_generation: i64,
    attempt: i32,
}

impl StorageUploadHandler {
    pub fn new(pool: PgPool, bus: Bus, storage_url: Url, max_track_duration_ms: i32) -> Self {
        Self {
            queue: JobRepository::new(pool.clone(), "storage-upload".to_owned()),
            pool,
            bus,
            storage_url,
            max_track_duration_ms,
        }
    }

    pub async fn accept(
        &self,
        payload: StorageTrackUploaded,
        delivery: DeliveryContext,
    ) -> JobResult {
        let uploaded = validate_upload(payload).map_err(JobError::permanent)?;
        let applied = apply_upload(
            &self.pool,
            &self.queue,
            &uploaded,
            &delivery,
            self.max_track_duration_ms,
        )
        .await?;
        if !applied.applied {
            tracing::debug!(
                track = %applied.sc_track_id,
                sequence = delivery.stream_sequence,
                "duplicate or stale storage upload was ignored"
            );
            return Ok(());
        }
        if applied.too_long {
            tracing::info!(
                track = %applied.sc_track_id,
                generation = applied.generation,
                "stored audio exceeded the duration limit"
            );
        } else {
            tracing::info!(
                track = %applied.sc_track_id,
                generation = applied.generation,
                "stored audio accepted"
            );
        }
        Ok(())
    }

    pub async fn dispatch_audio(&self, payload: StoredAudioDispatchPayload) -> JobResult {
        let payload = validate_dispatch(payload).map_err(JobError::permanent)?;
        let attempt = sqlx::query_file_scalar!(
            "queries/indexing/storage/dispatch_audio.sql",
            &payload.sc_track_id,
            payload.uploaded_generation
        )
        .fetch_one(&self.pool)
        .await
        .map_err(JobError::retryable)?;
        let Some(attempt) = attempt else {
            return Ok(());
        };

        self.publish_audio_index(&AudioDispatch {
            sc_track_id: payload.sc_track_id,
            upload_generation: payload.uploaded_generation,
            attempt,
        })
        .await
    }

    pub async fn reopen_audio_dispatches(
        &self,
        batch: i64,
        cooldown_seconds: i64,
        max_attempts: i32,
    ) -> JobResult {
        let reopened = sqlx::query_file_as!(
            AudioDispatch,
            "queries/indexing/reopen_dispatches.sql",
            batch,
            cooldown_seconds,
            max_attempts
        )
        .fetch_all(&self.pool)
        .await
        .map_err(JobError::retryable)?;

        let mut first_failure = None;
        for dispatch in reopened {
            tracing::info!(
                track = %dispatch.sc_track_id,
                generation = dispatch.upload_generation,
                attempt = dispatch.attempt,
                "audio index dispatch was reopened with a new attempt"
            );
            if let Err(error) = self.publish_audio_index(&dispatch).await {
                first_failure.get_or_insert(error);
            }
        }
        first_failure.map_or(Ok(()), Err)
    }

    async fn publish_audio_index(&self, dispatch: &AudioDispatch) -> JobResult {
        let published = match audio_index_request(&self.storage_url, dispatch) {
            Ok(request) => self
                .bus
                .publish_dedup(INDEX_AUDIO, &request, &audio_index_message_id(dispatch))
                .await
                .map_err(JobError::retryable),
            Err(error) => Err(JobError::permanent(error)),
        };
        if published.is_err() {
            abandon_audio_dispatch(&self.pool, dispatch).await;
        }
        published
    }
}

fn audio_index_request(
    storage_url: &Url,
    dispatch: &AudioDispatch,
) -> anyhow::Result<AudioIndexRequest> {
    Ok(AudioIndexRequest {
        s3_url: canonical_storage_url(storage_url, &dispatch.sc_track_id)?,
        sc_track_id: dispatch.sc_track_id.clone(),
        upload_generation: dispatch.upload_generation,
        attempt: i64::from(dispatch.attempt),
    })
}

fn audio_index_message_id(dispatch: &AudioDispatch) -> String {
    format!(
        "storage-audio:{}:{}:{}",
        dispatch.sc_track_id, dispatch.upload_generation, dispatch.attempt
    )
}

async fn abandon_audio_dispatch(pool: &PgPool, dispatch: &AudioDispatch) {
    let released = sqlx::query_file!(
        "queries/indexing/storage/abandon_audio_dispatch.sql",
        &dispatch.sc_track_id,
        dispatch.upload_generation,
        dispatch.attempt
    )
    .execute(pool)
    .await;
    match released {
        Ok(_) => tracing::warn!(
            track = %dispatch.sc_track_id,
            generation = dispatch.upload_generation,
            attempt = dispatch.attempt,
            "audio index dispatch was not published and waits to be reissued"
        ),
        Err(error) => tracing::error!(
            track = %dispatch.sc_track_id,
            generation = dispatch.upload_generation,
            attempt = dispatch.attempt,
            %error,
            "audio index dispatch could not release its unpublished claim"
        ),
    }
}

async fn apply_upload(
    pool: &PgPool,
    queue: &JobRepository,
    upload: &UploadedAudio,
    delivery: &DeliveryContext,
    max_track_duration_ms: i32,
) -> JobResult<AppliedUpload> {
    let stream_sequence = i64::try_from(delivery.stream_sequence)
        .map_err(|_| JobError::permanent(anyhow::anyhow!("NATS sequence is out of range")))?;
    let mut transaction = pool.begin().await.map_err(JobError::retryable)?;
    let outcome = sqlx::query_file!(
        "queries/indexing/storage/upload.sql",
        &delivery.consumer,
        &delivery.stream,
        stream_sequence,
        delivery.published_at,
        &upload.sc_track_id,
        upload.quality,
        max_track_duration_ms
    )
    .fetch_one(&mut *transaction)
    .await
    .map_err(JobError::retryable)?;

    if !outcome.known {
        transaction.rollback().await.map_err(JobError::retryable)?;
        return Err(JobError::retryable(anyhow::anyhow!(
            "storage upload references unknown track {}",
            upload.sc_track_id
        )));
    }
    if !outcome.accepted || !outcome.advanced {
        transaction.commit().await.map_err(JobError::retryable)?;
        return Ok(AppliedUpload {
            sc_track_id: upload.sc_track_id.clone(),
            generation: 0,
            too_long: false,
            applied: false,
        });
    }
    if !outcome.updated {
        transaction.rollback().await.map_err(JobError::retryable)?;
        return Err(JobError::retryable(anyhow::anyhow!(
            "storage upload cursor advanced without updating track {}",
            upload.sc_track_id
        )));
    }
    let generation = outcome.uploaded_generation.ok_or_else(|| {
        JobError::retryable(anyhow::anyhow!(
            "storage upload generation is missing for track {}",
            upload.sc_track_id
        ))
    })?;
    let payload = StoredAudioDispatchPayload {
        sc_track_id: upload.sc_track_id.clone(),
        uploaded_generation: generation,
    };
    if outcome.dispatch_audio {
        enqueue_dispatch(
            queue,
            &mut transaction,
            JobKind::DispatchAudioIndex,
            &payload,
        )
        .await?;
    }
    if outcome.dispatch_transcription {
        enqueue_dispatch(
            queue,
            &mut transaction,
            JobKind::DispatchTranscription,
            &payload,
        )
        .await?;
    }
    transaction.commit().await.map_err(JobError::retryable)?;
    Ok(AppliedUpload {
        sc_track_id: upload.sc_track_id.clone(),
        generation,
        too_long: outcome.too_long,
        applied: true,
    })
}

async fn enqueue_dispatch(
    queue: &JobRepository,
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    kind: JobKind,
    payload: &StoredAudioDispatchPayload,
) -> JobResult {
    let dedup_key = payload.sc_track_id.clone();
    let payload = serde_json::to_value(Versioned::V1(payload)).map_err(JobError::permanent)?;
    let job = NewJob {
        id: Uuid::now_v7(),
        kind,
        dedup_key: Some(dedup_key),
        payload,
        priority: DISPATCH_PRIORITY,
        max_attempts: MAX_ATTEMPTS,
        available_at: Utc::now(),
    };
    queue
        .enqueue_in(transaction, &job)
        .await
        .map_err(queue_error)
}

fn validate_upload(payload: StorageTrackUploaded) -> anyhow::Result<UploadedAudio> {
    let sc_track_id = normalize_track_id(&payload.sc_track_id)?;
    validate_storage_url(&payload.storage_url)?;
    let quality = match payload.quality.as_deref() {
        None => None,
        Some("sq") => Some("sq"),
        Some("hq") => Some("hq"),
        Some(_) => anyhow::bail!("storage upload has an invalid quality"),
    };
    Ok(UploadedAudio {
        sc_track_id,
        quality,
    })
}

fn canonical_storage_url(storage_url: &Url, sc_track_id: &str) -> anyhow::Result<String> {
    let mut url = storage_url.clone();
    let filename = format!("soundcloud_tracks_{sc_track_id}.m4a");
    let mut path = url
        .path_segments_mut()
        .map_err(|_| anyhow::anyhow!("storage URL cannot contain path segments"))?;
    path.extend(["redirect", filename.as_str()]);
    drop(path);
    validate_storage_url(url.as_str())
}

fn validate_dispatch(
    payload: StoredAudioDispatchPayload,
) -> anyhow::Result<StoredAudioDispatchPayload> {
    ensure!(
        payload.uploaded_generation > 0,
        "storage generation must be positive"
    );
    Ok(StoredAudioDispatchPayload {
        sc_track_id: normalize_track_id(&payload.sc_track_id)?,
        uploaded_generation: payload.uploaded_generation,
    })
}

fn normalize_track_id(value: &str) -> anyhow::Result<String> {
    let value = value.strip_prefix("soundcloud:tracks:").unwrap_or(value);
    let point_id = value
        .parse::<u64>()
        .map_err(|_| anyhow::anyhow!("storage upload has an invalid track id"))?;
    ensure!(
        point_id > 0 && point_id.to_string() == value,
        "storage upload has a non-canonical track id"
    );
    Ok(value.to_owned())
}

fn validate_storage_url(value: &str) -> anyhow::Result<String> {
    ensure!(
        !value.is_empty() && value.len() <= MAX_STORAGE_URL_BYTES,
        "storage upload has an invalid URL length"
    );
    let url =
        Url::parse(value).map_err(|_| anyhow::anyhow!("storage upload has an invalid URL"))?;
    ensure!(
        matches!(url.scheme(), "http" | "https")
            && url.host_str().is_some()
            && url.username().is_empty()
            && url.password().is_none()
            && url.fragment().is_none(),
        "storage upload has an unsafe URL"
    );
    Ok(url.to_string())
}

fn queue_error(error: QueueError) -> JobError {
    match error {
        QueueError::Database(error) => JobError::retryable(error),
        error => JobError::permanent(error),
    }
}
