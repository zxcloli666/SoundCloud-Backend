#[cfg(test)]
#[path = "dispatch_tests.rs"]
mod tests;

use anyhow::ensure;
use backend_contracts::StoredAudioDispatchPayload;
use backend_contracts::pipeline::{
    MAX_TEXT_BYTES, MAX_URL_CHARS, TRANSCRIBE_AUDIO, TranscriptionMode, TranscriptionRequest,
};
use sqlx::PgPool;
use url::Url;

use crate::bus::Bus;
use crate::queue::{JobError, JobResult};

use super::text::{reference_text, wire_language};

const UNUSABLE_REFERENCE: &str = "reference_text_unusable";

pub struct TranscriptionDispatcher {
    pool: PgPool,
    bus: Bus,
    storage_url: Url,
    enabled: bool,
}

struct TranscriptionDispatch {
    attempt: i64,
    plain_text: String,
    language: Option<String>,
}

impl TranscriptionDispatcher {
    pub fn new(pool: PgPool, bus: Bus, storage_url: Url, enabled: bool) -> Self {
        Self {
            pool,
            bus,
            storage_url,
            enabled,
        }
    }

    pub async fn dispatch_transcription(&self, payload: StoredAudioDispatchPayload) -> JobResult {
        if !self.enabled {
            tracing::debug!(
                track = %payload.sc_track_id,
                generation = payload.uploaded_generation,
                "transcription dispatch is switched off"
            );
            return Ok(());
        }
        let Some(request) = prepare(&self.pool, &self.storage_url, payload).await? else {
            return Ok(());
        };
        if let Err(error) = self
            .bus
            .publish_dedup(TRANSCRIBE_AUDIO, &request, &message_id(&request))
            .await
        {
            abandon(&self.pool, &request).await;
            return Err(JobError::retryable(error));
        }
        tracing::info!(
            track = %request.sc_track_id,
            generation = request.upload_generation,
            attempt = request.attempt,
            "transcription dispatched"
        );
        Ok(())
    }
}

async fn prepare(
    pool: &PgPool,
    storage_url: &Url,
    payload: StoredAudioDispatchPayload,
) -> JobResult<Option<TranscriptionRequest>> {
    let payload = validate(payload).map_err(JobError::permanent)?;
    let audio_url = audio_url(storage_url, &payload.sc_track_id).map_err(JobError::permanent)?;
    let mut transaction = pool.begin().await.map_err(JobError::retryable)?;
    let dispatch = sqlx::query_file_as!(
        TranscriptionDispatch,
        "queries/lyrics/prepare_transcription.sql",
        &payload.sc_track_id,
        payload.uploaded_generation
    )
    .fetch_optional(&mut *transaction)
    .await
    .map_err(JobError::retryable)?;
    let Some(dispatch) = dispatch else {
        transaction.commit().await.map_err(JobError::retryable)?;
        return Ok(None);
    };

    let Some(reference) = reference_text(&dispatch.plain_text, MAX_TEXT_BYTES as usize) else {
        sqlx::query_file!(
            "queries/lyrics/quarantine_transcription_request.sql",
            &payload.sc_track_id,
            payload.uploaded_generation,
            dispatch.attempt,
            UNUSABLE_REFERENCE
        )
        .execute(&mut *transaction)
        .await
        .map_err(JobError::retryable)?;
        transaction.commit().await.map_err(JobError::retryable)?;
        tracing::warn!(
            track = %payload.sc_track_id,
            generation = payload.uploaded_generation,
            "transcription reference text has no whole line within the wire limit"
        );
        return Ok(None);
    };
    transaction.commit().await.map_err(JobError::retryable)?;

    Ok(Some(TranscriptionRequest {
        sc_track_id: payload.sc_track_id,
        upload_generation: payload.uploaded_generation,
        attempt: dispatch.attempt,
        audio_url,
        reference_text: reference.text,
        reference_lines_total: reference.lines_total,
        language: wire_language(dispatch.language.as_deref()),
        mode: TranscriptionMode::Align,
    }))
}

async fn abandon(pool: &PgPool, request: &TranscriptionRequest) {
    let released = sqlx::query_file!(
        "queries/lyrics/abandon_transcription_dispatch.sql",
        &request.sc_track_id,
        request.upload_generation,
        request.attempt
    )
    .execute(pool)
    .await;
    match released {
        Ok(_) => tracing::warn!(
            track = %request.sc_track_id,
            generation = request.upload_generation,
            attempt = request.attempt,
            "transcription dispatch was not published and waits for a reopen"
        ),
        Err(error) => tracing::error!(
            track = %request.sc_track_id,
            generation = request.upload_generation,
            attempt = request.attempt,
            %error,
            "unpublished transcription dispatch could not be released"
        ),
    }
}

fn message_id(request: &TranscriptionRequest) -> String {
    format!(
        "transcribe:{}:{}:{}",
        request.sc_track_id, request.upload_generation, request.attempt
    )
}

fn validate(payload: StoredAudioDispatchPayload) -> anyhow::Result<StoredAudioDispatchPayload> {
    ensure!(
        payload.uploaded_generation > 0,
        "transcription dispatch needs a positive upload generation"
    );
    let id = payload
        .sc_track_id
        .parse::<u64>()
        .map_err(|_| anyhow::anyhow!("transcription dispatch has an invalid track id"))?;
    ensure!(
        id > 0 && id.to_string() == payload.sc_track_id,
        "transcription dispatch has a non-canonical track id"
    );
    Ok(payload)
}

fn audio_url(storage_url: &Url, sc_track_id: &str) -> anyhow::Result<String> {
    let mut url = storage_url.clone();
    let filename = format!("soundcloud_tracks_{sc_track_id}.m4a");
    url.path_segments_mut()
        .map_err(|_| anyhow::anyhow!("storage URL cannot contain path segments"))?
        .extend(["redirect", filename.as_str()]);
    ensure!(
        matches!(url.scheme(), "http" | "https")
            && url.host_str().is_some()
            && url.username().is_empty()
            && url.password().is_none()
            && url.fragment().is_none(),
        "storage URL is not a plain HTTP(S) address"
    );
    let url = url.to_string();
    ensure!(
        url.chars().count() <= MAX_URL_CHARS as usize,
        "audio URL is longer than the wire allows"
    );
    Ok(url)
}
