#[cfg(test)]
#[path = "embedding_result_tests.rs"]
mod db_tests;

use std::sync::Arc;

use anyhow::ensure;
use backend_contracts::pipeline::{
    LyricsEmbeddingRequest, LyricsEmbeddingResult, MAX_REQUEST_ID_CHARS, MAX_TEXT_BYTES,
};
use backend_contracts::reasons::{WorkerReason, WorkerStatus, outcome_rank};
use backend_contracts::vector_store::TRACKS_LYRICS_DIMENSIONS;
use backend_contracts::worker_contract::WorkerLane;
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use uuid::Uuid;

use crate::bus::DeliveryContext;
use crate::queue::{JobError, JobResult};

use super::text::{embedding_text, wire_language};
use super::vectors::LyricsVectorStore;

const RESULT_CLAIM_SECONDS: i64 = 30;
const MAX_EMBEDDING_REOPENS: i32 = 3;

pub struct EmbeddingResultHandler {
    pool: PgPool,
    vectors: Arc<dyn LyricsVectorStore>,
}

#[derive(Debug, PartialEq)]
struct ValidatedEmbedding {
    sc_track_id: String,
    point_id: u64,
    request_id: String,
    rank: i16,
    outcome: EmbeddingOutcome,
}

#[derive(Debug, PartialEq)]
enum EmbeddingOutcome {
    Vector {
        vector: Vec<f32>,
        language: Option<String>,
    },
    Skipped {
        reason: WorkerReason,
        language: Option<String>,
    },
    Failed {
        reason: WorkerReason,
        reopenable: bool,
    },
}

impl EmbeddingOutcome {
    const fn result_kind(&self) -> &'static str {
        match self {
            Self::Vector { .. } => "vector",
            Self::Skipped { .. } => "skipped",
            Self::Failed {
                reopenable: true, ..
            } => "reopen",
            Self::Failed { .. } => "failed",
        }
    }

    const fn wire_status(&self) -> &'static str {
        match self {
            Self::Vector { .. } => "done",
            Self::Skipped { .. } => "skipped",
            Self::Failed {
                reopenable: true, ..
            } => "reopenable",
            Self::Failed { .. } => "failed",
        }
    }

    fn reason(&self) -> Option<&'static str> {
        match self {
            Self::Vector { .. } => None,
            Self::Skipped { reason, .. } | Self::Failed { reason, .. } => Some(reason.as_str()),
        }
    }

    fn language(&self) -> Option<&str> {
        match self {
            Self::Vector { language, .. } | Self::Skipped { language, .. } => language.as_deref(),
            Self::Failed { .. } => None,
        }
    }
}

enum Claim {
    Settled,
    Busy,
    Owned {
        lease_id: Uuid,
        request_matches: bool,
    },
}

struct Delivery<'a> {
    context: &'a DeliveryContext,
    stream_sequence: i64,
}

impl EmbeddingResultHandler {
    pub fn new(pool: PgPool, vectors: Arc<dyn LyricsVectorStore>) -> Self {
        Self { pool, vectors }
    }

    pub async fn finish(
        &self,
        result: LyricsEmbeddingResult,
        context: DeliveryContext,
    ) -> JobResult {
        let embedding = validate(result).map_err(JobError::permanent)?;
        let stream_sequence = i64::try_from(context.stream_sequence)
            .map_err(|_| JobError::permanent(anyhow::anyhow!("NATS sequence is out of range")))?;
        let delivery = Delivery {
            context: &context,
            stream_sequence,
        };
        let (lease_id, request_matches) = match self.claim(&embedding, &delivery).await? {
            Claim::Settled => return Ok(()),
            Claim::Busy => {
                return Err(JobError::retryable(anyhow::anyhow!(
                    "lyrics embedding result is already being applied"
                )));
            }
            Claim::Owned {
                lease_id,
                request_matches,
            } => (lease_id, request_matches),
        };
        if !request_matches {
            return self
                .quarantine(&embedding, &delivery, lease_id, "request_text_changed")
                .await;
        }
        if let EmbeddingOutcome::Vector { vector, language } = &embedding.outcome {
            self.store_vector(&embedding, vector, language.as_deref())
                .await?;
        }
        self.complete(&embedding, &delivery, lease_id).await
    }

    pub async fn apply_worker_lost(&self, stream_seq: u64, payload: &[u8]) -> JobResult {
        let request = serde_json::from_slice::<LyricsEmbeddingRequest>(payload)
            .map_err(JobError::permanent)?;
        canonical_point_id(&request.sc_track_id).map_err(JobError::permanent)?;
        let settled = sqlx::query_file_scalar!(
            "queries/lyrics/apply_embedding_worker_lost.sql",
            &request.sc_track_id,
            &request.request_id,
            WorkerReason::WorkerLost.as_str(),
            MAX_EMBEDDING_REOPENS
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(JobError::retryable)?;
        match settled {
            Some(status) => tracing::warn!(
                track = %request.sc_track_id,
                request = %request.request_id,
                stream_seq,
                status,
                "lyrics embedding worker was lost; the request waits for a reopen"
            ),
            None => tracing::debug!(
                track = %request.sc_track_id,
                request = %request.request_id,
                stream_seq,
                "lost lyrics embedding request is no longer the open one"
            ),
        }
        Ok(())
    }

    async fn claim(
        &self,
        embedding: &ValidatedEmbedding,
        delivery: &Delivery<'_>,
    ) -> JobResult<Claim> {
        let lease_id = Uuid::now_v7();
        let kind = embedding.outcome.result_kind();
        let outcome = sqlx::query_file!(
            "queries/lyrics/claim_embedding_result.sql",
            &delivery.context.consumer,
            &delivery.context.stream,
            delivery.stream_sequence,
            delivery.context.published_at,
            &embedding.sc_track_id,
            kind,
            lease_id,
            RESULT_CLAIM_SECONDS,
            &embedding.request_id,
            embedding.rank
        )
        .fetch_one(&self.pool)
        .await
        .map_err(JobError::retryable)?;
        if outcome.kind_mismatch {
            return Err(JobError::permanent(anyhow::anyhow!(
                "lyrics embedding delivery changed result kind"
            )));
        }
        if outcome.claimed {
            if outcome.result_kind.as_deref() != Some(kind) {
                return Err(JobError::permanent(anyhow::anyhow!(
                    "lyrics embedding result claim kind does not match payload"
                )));
            }
            let current_text = outcome
                .lyrics_current
                .then(|| {
                    embedding_text(
                        outcome.plain_text.as_deref(),
                        outcome.synced_lrc.as_deref(),
                        MAX_TEXT_BYTES as usize,
                    )
                })
                .flatten();
            let request_matches = match (current_text, outcome.request_sha256) {
                (Some(text), Some(sha256)) => {
                    Sha256::digest(text.as_bytes()).as_slice() == sha256.as_slice()
                }
                _ => false,
            };
            return Ok(Claim::Owned {
                lease_id,
                request_matches,
            });
        }
        if outcome.busy {
            return Ok(Claim::Busy);
        }
        if outcome.settled {
            tracing::debug!(
                track = %embedding.sc_track_id,
                request = %embedding.request_id,
                "lyrics embedding result does not belong to the open request"
            );
            return Ok(Claim::Settled);
        }
        Err(JobError::retryable(anyhow::anyhow!(
            "lyrics embedding result was neither claimed nor settled"
        )))
    }

    async fn store_vector(
        &self,
        embedding: &ValidatedEmbedding,
        vector: &[f32],
        language: Option<&str>,
    ) -> JobResult {
        self.vectors
            .upsert_lyrics(embedding.point_id, &embedding.request_id, language, vector)
            .await
            .map_err(JobError::retryable)
    }

    async fn complete(
        &self,
        embedding: &ValidatedEmbedding,
        delivery: &Delivery<'_>,
        lease_id: Uuid,
    ) -> JobResult {
        let outcome = sqlx::query_file!(
            "queries/lyrics/complete_embedding_result.sql",
            &delivery.context.consumer,
            &delivery.context.stream,
            delivery.stream_sequence,
            delivery.context.published_at,
            &embedding.sc_track_id,
            lease_id,
            embedding.outcome.wire_status(),
            embedding.outcome.reason(),
            embedding.outcome.language(),
            MAX_EMBEDDING_REOPENS
        )
        .fetch_one(&self.pool)
        .await
        .map_err(JobError::retryable)?;
        if outcome.already_settled {
            return Ok(());
        }
        if !outcome.owned {
            return Err(JobError::retryable(anyhow::anyhow!(
                "lyrics embedding result lost its fenced claim before completion"
            )));
        }
        if !outcome.lyrics_current {
            return self
                .quarantine(embedding, delivery, lease_id, "lyrics_changed_before_apply")
                .await;
        }
        if !outcome.accepted {
            return Err(JobError::retryable(anyhow::anyhow!(
                "lyrics embedding result was completed without a receipt"
            )));
        }
        tracing::info!(
            track = %embedding.sc_track_id,
            request = %embedding.request_id,
            status = outcome.settled_status.as_deref().unwrap_or_default(),
            reason = embedding.outcome.reason().unwrap_or_default(),
            "lyrics embedding result applied"
        );
        Ok(())
    }

    async fn quarantine(
        &self,
        embedding: &ValidatedEmbedding,
        delivery: &Delivery<'_>,
        lease_id: Uuid,
        reason: &str,
    ) -> JobResult {
        let outcome = sqlx::query_file!(
            "queries/lyrics/quarantine_embedding_result.sql",
            &delivery.context.consumer,
            &delivery.context.stream,
            delivery.stream_sequence,
            delivery.context.published_at,
            &embedding.sc_track_id,
            lease_id,
            reason,
            MAX_EMBEDDING_REOPENS
        )
        .fetch_one(&self.pool)
        .await
        .map_err(JobError::retryable)?;
        if outcome.already_settled {
            return Ok(());
        }
        if !outcome.owned || !outcome.wire_quarantined || !outcome.accepted {
            return Err(JobError::retryable(anyhow::anyhow!(
                "lyrics embedding result lost its fenced claim before quarantine"
            )));
        }
        tracing::warn!(
            track = %embedding.sc_track_id,
            request = %embedding.request_id,
            reason,
            reembed = outcome.cache_released,
            "lyrics embedding result no longer matches its request"
        );
        Ok(())
    }
}

fn canonical_point_id(sc_track_id: &str) -> anyhow::Result<u64> {
    let point_id = sc_track_id
        .parse::<u64>()
        .map_err(|_| anyhow::anyhow!("lyrics track id {sc_track_id:?} is invalid"))?;
    ensure!(
        point_id > 0 && point_id.to_string() == sc_track_id,
        "lyrics track id {sc_track_id:?} is not canonical"
    );
    Ok(point_id)
}

fn validate(result: LyricsEmbeddingResult) -> anyhow::Result<ValidatedEmbedding> {
    let point_id = canonical_point_id(&result.sc_track_id)?;
    ensure!(
        !result.request_id.is_empty()
            && result.request_id.chars().count() <= MAX_REQUEST_ID_CHARS as usize
            && !result.request_id.chars().any(char::is_control),
        "lyrics result has an invalid request id"
    );
    let language = wire_language(result.language.as_deref());
    let rank = i16::from(outcome_rank(result.status, result.reason));
    let outcome = match (result.status, result.reason) {
        (WorkerStatus::Ok, None) => EmbeddingOutcome::Vector {
            vector: checked_vector(result.vector)?,
            language,
        },
        (WorkerStatus::Ok, Some(_)) => anyhow::bail!("an embedded lyrics result carries a reason"),
        (_, None) => anyhow::bail!("a lyrics result other than ok needs a reason"),
        (status, Some(reason)) => {
            ensure!(
                reason.status() == status,
                "lyrics reason {} does not belong to status {}",
                reason.as_str(),
                status.as_str()
            );
            ensure!(
                reason.is_published_on(WorkerLane::Lyrics),
                "lyrics lane never publishes {}",
                reason.as_str()
            );
            match status {
                WorkerStatus::Empty => EmbeddingOutcome::Skipped { reason, language },
                _ => EmbeddingOutcome::Failed {
                    reason,
                    reopenable: reason.is_reopenable_on(WorkerLane::Lyrics),
                },
            }
        }
    };
    Ok(ValidatedEmbedding {
        sc_track_id: result.sc_track_id,
        point_id,
        request_id: result.request_id,
        rank,
        outcome,
    })
}

fn checked_vector(vector: Option<Vec<f32>>) -> anyhow::Result<Vec<f32>> {
    let vector = vector.ok_or_else(|| anyhow::anyhow!("lyrics result has no vector"))?;
    let dimensions = usize::try_from(TRACKS_LYRICS_DIMENSIONS)
        .map_err(|_| anyhow::anyhow!("lyrics dimensions do not fit this platform"))?;
    ensure!(
        vector.len() == dimensions,
        "lyrics vector has {} values, expected {dimensions}",
        vector.len()
    );
    ensure!(
        vector.iter().all(|value| value.is_finite()),
        "lyrics vector contains a non-finite value"
    );
    Ok(vector)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use backend_contracts::pipeline::Producer;

    use super::*;

    fn result(status: WorkerStatus, reason: Option<WorkerReason>) -> LyricsEmbeddingResult {
        LyricsEmbeddingResult {
            sc_track_id: "42".to_owned(),
            request_id: "lyr:42:1".to_owned(),
            status,
            reason,
            detail: None,
            producer: Producer {
                worker_id: "gpu-main".to_owned(),
                build: "test".to_owned(),
                models: BTreeMap::new(),
                sync_version: None,
            },
            vector: (status == WorkerStatus::Ok).then(|| vec![0.5; 1024]),
            language: Some("EN".to_owned()),
        }
    }

    #[test]
    fn a_complete_embedding_is_a_vector_with_a_wire_language() -> anyhow::Result<()> {
        let embedding = validate(result(WorkerStatus::Ok, None))?;

        assert_eq!(embedding.point_id, 42);
        assert_eq!(embedding.outcome.result_kind(), "vector");
        assert_eq!(embedding.outcome.language(), Some("en"));
        Ok(())
    }

    #[test]
    fn missing_and_wrong_sized_vectors_are_invalid() {
        let mut missing = result(WorkerStatus::Ok, None);
        missing.vector = None;
        let mut short = result(WorkerStatus::Ok, None);
        short.vector = Some(vec![0.5; 8]);

        assert!(validate(missing).is_err());
        assert!(validate(short).is_err());
    }

    #[test]
    fn empty_text_is_skipped_and_engine_restarts_reopen() -> anyhow::Result<()> {
        let empty = validate(result(WorkerStatus::Empty, Some(WorkerReason::EmptyText)))?;
        let restarted = validate(result(
            WorkerStatus::Failed,
            Some(WorkerReason::EngineRestarted),
        ))?;
        let too_long = validate(result(
            WorkerStatus::Failed,
            Some(WorkerReason::TextTooLongForModel),
        ))?;

        assert_eq!(empty.outcome.wire_status(), "skipped");
        assert_eq!(restarted.outcome.wire_status(), "reopenable");
        assert_eq!(restarted.outcome.result_kind(), "reopen");
        assert_eq!(too_long.outcome.wire_status(), "failed");
        Ok(())
    }

    #[test]
    fn reasons_foreign_to_the_lyrics_lane_are_invalid() {
        assert!(
            validate(result(
                WorkerStatus::Failed,
                Some(WorkerReason::PublicNodeTimeout)
            ))
            .is_err()
        );
        assert!(
            validate(result(
                WorkerStatus::Empty,
                Some(WorkerReason::LowConfidence)
            ))
            .is_err()
        );
        assert!(validate(result(WorkerStatus::Failed, None)).is_err());
    }
}
