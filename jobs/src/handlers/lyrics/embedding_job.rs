#[cfg(test)]
#[path = "embedding_job_tests.rs"]
mod db_tests;

use anyhow::ensure;
use backend_contracts::LyricsEmbedPayload;
use backend_contracts::pipeline::{EMBED_LYRICS, LyricsEmbeddingRequest, MAX_TEXT_BYTES};
use sha2::{Digest, Sha256};
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

use crate::bus::Bus;
use crate::queue::{JobError, JobResult};

use super::text::{embedding_text, wire_language};

pub struct LyricsEmbeddingJob {
    pool: PgPool,
    bus: Bus,
    enabled: bool,
}

struct EmbeddingCandidate {
    plain_text: Option<String>,
    synced_lrc: Option<String>,
    language: Option<String>,
    content_generation: i64,
    embedding_state: String,
    request_id: Option<String>,
    request_text: Option<String>,
    request_language: Option<String>,
    request_published: Option<bool>,
}

enum Step {
    Nothing,
    Publish(LyricsEmbeddingRequest),
    Acknowledge(String),
}

impl LyricsEmbeddingJob {
    pub fn new(pool: PgPool, bus: Bus, enabled: bool) -> Self {
        Self { pool, bus, enabled }
    }

    pub async fn run(&self, payload: LyricsEmbedPayload) -> JobResult {
        let sc_track_id = canonical_track_id(&payload.sc_track_id).map_err(JobError::permanent)?;
        if !self.enabled {
            release(&self.pool, &sc_track_id).await?;
            tracing::debug!(track = %sc_track_id, "lyrics embedding dispatch is switched off");
            return Ok(());
        }
        match prepare(&self.pool, &sc_track_id).await? {
            Step::Nothing => Ok(()),
            Step::Acknowledge(request_id) => {
                acknowledge(&self.pool, &sc_track_id, &request_id).await
            }
            Step::Publish(request) => {
                self.bus
                    .publish_dedup(EMBED_LYRICS, &request, &message_id(&request))
                    .await
                    .map_err(JobError::retryable)?;
                acknowledge(&self.pool, &sc_track_id, &request.request_id).await
            }
        }
    }
}

async fn prepare(pool: &PgPool, sc_track_id: &str) -> JobResult<Step> {
    let mut transaction = pool.begin().await.map_err(JobError::retryable)?;
    let candidate = sqlx::query_file_as!(
        EmbeddingCandidate,
        "queries/lyrics/load_embedding_request.sql",
        sc_track_id
    )
    .fetch_optional(&mut *transaction)
    .await
    .map_err(JobError::retryable)?;
    let step = match candidate {
        None => Step::Nothing,
        Some(candidate) if candidate.embedding_state == "queued" => {
            open_request(&mut transaction, sc_track_id, candidate).await?
        }
        Some(candidate) => resume_request(&mut transaction, sc_track_id, candidate).await?,
    };
    transaction.commit().await.map_err(JobError::retryable)?;
    Ok(step)
}

async fn open_request(
    transaction: &mut Transaction<'_, Postgres>,
    sc_track_id: &str,
    candidate: EmbeddingCandidate,
) -> JobResult<Step> {
    let Some(text) = embedding_text(
        candidate.plain_text.as_deref(),
        candidate.synced_lrc.as_deref(),
        MAX_TEXT_BYTES as usize,
    ) else {
        sqlx::query_file!(
            "queries/lyrics/skip_embedding.sql",
            sc_track_id,
            candidate.content_generation
        )
        .execute(&mut **transaction)
        .await
        .map_err(JobError::retryable)?;
        return Ok(Step::Nothing);
    };
    let request = LyricsEmbeddingRequest {
        sc_track_id: sc_track_id.to_owned(),
        request_id: format!("lyr:{sc_track_id}:{}", Uuid::now_v7().simple()),
        language: wire_language(candidate.language.as_deref()),
        text,
    };
    let sha256 = Sha256::digest(request.text.as_bytes()).to_vec();
    let opened = sqlx::query_file_scalar!(
        "queries/lyrics/open_embedding_request.sql",
        sc_track_id,
        candidate.content_generation,
        &request.text,
        request.language.as_deref(),
        &sha256,
        &request.request_id
    )
    .fetch_one(&mut **transaction)
    .await
    .map_err(JobError::retryable)?;
    if !opened {
        tracing::debug!(track = %sc_track_id, "lyrics embedding request is already open");
        return Ok(Step::Nothing);
    }
    Ok(Step::Publish(request))
}

async fn resume_request(
    transaction: &mut Transaction<'_, Postgres>,
    sc_track_id: &str,
    candidate: EmbeddingCandidate,
) -> JobResult<Step> {
    match (
        candidate.request_id,
        candidate.request_text,
        candidate.request_published,
    ) {
        (Some(request_id), _, Some(true)) => Ok(Step::Acknowledge(request_id)),
        (Some(request_id), Some(text), _) => Ok(Step::Publish(LyricsEmbeddingRequest {
            sc_track_id: sc_track_id.to_owned(),
            request_id,
            text,
            language: candidate.request_language,
        })),
        _ => {
            sqlx::query_file!("queries/lyrics/release_embedding.sql", sc_track_id)
                .execute(&mut **transaction)
                .await
                .map_err(JobError::retryable)?;
            Ok(Step::Nothing)
        }
    }
}

async fn acknowledge(pool: &PgPool, sc_track_id: &str, request_id: &str) -> JobResult {
    sqlx::query_file!(
        "queries/lyrics/mark_embedding_published.sql",
        sc_track_id,
        request_id
    )
    .execute(pool)
    .await
    .map_err(JobError::retryable)?;
    Ok(())
}

async fn release(pool: &PgPool, sc_track_id: &str) -> JobResult {
    sqlx::query_file!("queries/lyrics/release_embedding.sql", sc_track_id)
        .execute(pool)
        .await
        .map_err(JobError::retryable)?;
    Ok(())
}

fn message_id(request: &LyricsEmbeddingRequest) -> String {
    format!("embed:{}:{}", request.sc_track_id, request.request_id)
}

fn canonical_track_id(value: &str) -> anyhow::Result<String> {
    let value = value.strip_prefix("soundcloud:tracks:").unwrap_or(value);
    let id = value
        .parse::<u64>()
        .map_err(|_| anyhow::anyhow!("lyrics embedding job has an invalid track id"))?;
    ensure!(
        id > 0 && id.to_string() == value,
        "lyrics embedding job has a non-canonical track id"
    );
    Ok(value.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_ids_accept_bare_and_soundcloud_urns() {
        assert_eq!(canonical_track_id("42").ok().as_deref(), Some("42"));
        assert_eq!(
            canonical_track_id("soundcloud:tracks:42").ok().as_deref(),
            Some("42")
        );
        assert!(canonical_track_id("tracks:42").is_err());
        assert!(canonical_track_id("042").is_err());
    }

    #[test]
    fn the_message_id_carries_the_request_identity() {
        let request = LyricsEmbeddingRequest {
            sc_track_id: "42".to_owned(),
            request_id: "lyr:42:abc".to_owned(),
            text: "line".to_owned(),
            language: None,
        };

        assert_eq!(message_id(&request), "embed:42:lyr:42:abc");
    }
}
