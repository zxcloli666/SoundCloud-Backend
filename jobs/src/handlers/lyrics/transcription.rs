#[cfg(test)]
#[path = "transcription_tests.rs"]
mod db_tests;

use anyhow::ensure;
use backend_contracts::pipeline::{TranscriptionRequest, TranscriptionResult};
use backend_contracts::reasons::{WorkerReason, WorkerStatus, outcome_rank};
use backend_contracts::worker_contract::WorkerLane;
use sqlx::PgPool;

use crate::bus::DeliveryContext;
use crate::queue::{JobError, JobResult};

use super::text::wire_language;

const MAX_LYRICS_BYTES: usize = 800_000;
const MAX_SYNC_VERSION_BYTES: usize = 128;

pub struct TranscriptionResultHandler {
    pool: PgPool,
}

#[derive(Debug, PartialEq)]
struct ValidatedResult {
    sc_track_id: String,
    upload_generation: i64,
    attempt: i64,
    status: WorkerStatus,
    outcome: Outcome,
    reason: Option<WorkerReason>,
    sync_version: String,
    synced_lrc: Option<String>,
    confidence: Option<f64>,
    placed_share: Option<f64>,
    aligned_share: Option<f64>,
    lines_total: Option<i32>,
    lines_unplaced: Option<i32>,
    language: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Outcome {
    Aligned,
    Empty,
    Rejected,
    Reopenable,
    Quarantined,
}

impl Outcome {
    fn of(status: WorkerStatus, reason: Option<WorkerReason>) -> Self {
        match (status, reason) {
            (WorkerStatus::Ok, _) => Self::Aligned,
            (WorkerStatus::Empty, _) => Self::Empty,
            (WorkerStatus::Rejected, _) => Self::Rejected,
            (WorkerStatus::Failed, Some(reason))
                if reason.is_reopenable_on(WorkerLane::Transcribe) =>
            {
                Self::Reopenable
            }
            _ => Self::Quarantined,
        }
    }

    const fn wire_status(self) -> &'static str {
        match self {
            Self::Aligned => "done",
            Self::Empty => "empty",
            Self::Rejected => "rejected",
            Self::Reopenable => "reopenable",
            Self::Quarantined => "quarantined",
        }
    }

    const fn track_state(self) -> &'static str {
        match self {
            Self::Aligned => "done",
            Self::Empty => "disabled",
            Self::Rejected => "rejected",
            Self::Reopenable => "pending",
            Self::Quarantined => "quarantined",
        }
    }
}

impl TranscriptionResultHandler {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub async fn apply_worker_lost(&self, stream_seq: u64, payload: &[u8]) -> JobResult {
        let request =
            serde_json::from_slice::<TranscriptionRequest>(payload).map_err(JobError::permanent)?;
        let sc_track_id = canonical_track_id(&request.sc_track_id).map_err(JobError::permanent)?;
        let reason = WorkerReason::WorkerLost;
        let rank = i16::from(outcome_rank(WorkerStatus::Failed, Some(reason)));
        let reopened = sqlx::query_file_scalar!(
            "queries/lyrics/apply_worker_lost.sql",
            &sc_track_id,
            request.upload_generation,
            request.attempt,
            reason.as_str(),
            rank
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(JobError::retryable)?;
        if reopened.is_some() {
            tracing::warn!(
                track = %sc_track_id,
                generation = request.upload_generation,
                attempt = request.attempt,
                stream_seq,
                "transcription worker was lost; the attempt waits for a reopen"
            );
        } else {
            tracing::debug!(
                track = %sc_track_id,
                generation = request.upload_generation,
                attempt = request.attempt,
                stream_seq,
                "lost transcription attempt already has a stronger outcome"
            );
        }
        Ok(())
    }

    pub async fn finish(
        &self,
        result: TranscriptionResult,
        delivery: DeliveryContext,
    ) -> JobResult {
        let result = validate(result).map_err(JobError::permanent)?;
        let stream_sequence = i64::try_from(delivery.stream_sequence)
            .map_err(|_| JobError::permanent(anyhow::anyhow!("NATS sequence is out of range")))?;
        let reason = result.reason.map(WorkerReason::as_str);
        let rank = i16::from(outcome_rank(result.status, result.reason));
        let mut transaction = self.pool.begin().await.map_err(JobError::retryable)?;
        let outcome = sqlx::query_file!(
            "queries/lyrics/apply_transcription.sql",
            &delivery.consumer,
            &delivery.stream,
            stream_sequence,
            delivery.published_at,
            &result.sc_track_id,
            result.upload_generation,
            result.attempt,
            result.outcome.wire_status(),
            result.outcome.track_state(),
            reason,
            rank,
            &result.sync_version,
            result.synced_lrc.as_deref(),
            result.confidence,
            result.placed_share,
            result.aligned_share,
            result.lines_total,
            result.lines_unplaced,
            result.language.as_deref(),
            is_released_sync_version(&result.sync_version)
        )
        .fetch_one(&mut *transaction)
        .await
        .map_err(JobError::retryable)?;

        if !outcome.known {
            transaction.rollback().await.map_err(JobError::retryable)?;
            return Err(JobError::retryable(anyhow::anyhow!(
                "transcription result references unknown track {}",
                result.sc_track_id
            )));
        }
        if outcome.deferred {
            transaction.rollback().await.map_err(JobError::retryable)?;
            return Err(JobError::retryable(anyhow::anyhow!(
                "transcription result waits for track {} to become servable",
                result.sc_track_id
            )));
        }
        transaction.commit().await.map_err(JobError::retryable)?;

        if !outcome.accepted {
            tracing::debug!(
                track = %result.sc_track_id,
                sequence = delivery.stream_sequence,
                "transcription result was already consumed"
            );
        } else if !outcome.applied {
            tracing::info!(
                track = %result.sc_track_id,
                generation = result.upload_generation,
                attempt = result.attempt,
                status = result.outcome.wire_status(),
                "transcription result does not match the current attempt"
            );
        } else {
            tracing::info!(
                track = %result.sc_track_id,
                generation = result.upload_generation,
                attempt = result.attempt,
                status = result.outcome.wire_status(),
                reason = reason.unwrap_or_default(),
                sync_version = %result.sync_version,
                aligned = outcome.aligned,
                "transcription result applied"
            );
        }
        Ok(())
    }
}

fn validate(result: TranscriptionResult) -> anyhow::Result<ValidatedResult> {
    let sc_track_id = canonical_track_id(&result.sc_track_id)?;
    ensure!(
        result.upload_generation > 0 && result.attempt > 0,
        "transcription result needs a positive generation and attempt"
    );
    ensure!(
        !result.sync_version.trim().is_empty()
            && result.sync_version.len() <= MAX_SYNC_VERSION_BYTES
            && !result.sync_version.chars().any(char::is_control),
        "transcription result has an invalid sync version"
    );
    let reason = validated_reason(result.status, result.reason)?;
    let outcome = Outcome::of(result.status, reason);
    let synced_lrc = match outcome {
        Outcome::Aligned => Some(synced_lyrics(result.synced_lrc)?),
        _ => None,
    };
    Ok(ValidatedResult {
        sc_track_id,
        upload_generation: result.upload_generation,
        attempt: result.attempt,
        status: result.status,
        outcome,
        reason,
        sync_version: result.sync_version,
        synced_lrc,
        confidence: share(result.confidence, "confidence")?,
        placed_share: share(result.placed_share, "placed share")?,
        aligned_share: share(result.aligned_share, "aligned share")?,
        lines_total: line_count(result.lines_total)?,
        lines_unplaced: line_count(result.lines_unplaced)?,
        language: wire_language(result.language.as_deref()),
    })
}

fn is_released_sync_version(value: &str) -> bool {
    let mut parts = value.split('.');
    let schema = parts
        .next()
        .and_then(|head| head.strip_prefix('s'))
        .is_some_and(|digits| {
            !digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit())
        });
    let revisions: Vec<&str> = parts.collect();
    schema
        && revisions.len() == 3
        && revisions.iter().all(|revision| {
            (1..=8).contains(&revision.len())
                && revision
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
        })
}

fn validated_reason(
    status: WorkerStatus,
    reason: Option<WorkerReason>,
) -> anyhow::Result<Option<WorkerReason>> {
    match (status, reason) {
        (WorkerStatus::Ok, None) => Ok(None),
        (WorkerStatus::Ok, Some(_)) => anyhow::bail!("an aligned transcription carries a reason"),
        (_, None) => anyhow::bail!("a transcription outcome other than ok needs a reason"),
        (status, Some(reason)) => {
            ensure!(
                reason.status() == status,
                "transcription reason {} does not belong to status {}",
                reason.as_str(),
                status.as_str()
            );
            ensure!(
                reason.is_published_on(WorkerLane::Transcribe),
                "transcription lane never publishes {}",
                reason.as_str()
            );
            Ok(Some(reason))
        }
    }
}

fn synced_lyrics(value: Option<String>) -> anyhow::Result<String> {
    let value = value.ok_or_else(|| anyhow::anyhow!("an aligned transcription has no lyrics"))?;
    ensure!(
        !value.trim().is_empty(),
        "an aligned transcription has blank lyrics"
    );
    ensure!(
        value.len() <= MAX_LYRICS_BYTES,
        "synced lyrics are too large"
    );
    ensure!(!value.contains('\0'), "synced lyrics contain a null byte");
    Ok(value)
}

fn share(value: Option<f64>, name: &str) -> anyhow::Result<Option<f64>> {
    ensure!(
        value.is_none_or(|value| value.is_finite() && (0.0..=1.0).contains(&value)),
        "transcription {name} is outside 0..1"
    );
    Ok(value)
}

fn line_count(value: Option<u32>) -> anyhow::Result<Option<i32>> {
    value
        .map(i32::try_from)
        .transpose()
        .map_err(|_| anyhow::anyhow!("transcription line count is out of range"))
}

fn canonical_track_id(value: &str) -> anyhow::Result<String> {
    let id = value
        .parse::<u64>()
        .map_err(|_| anyhow::anyhow!("transcription has an invalid track id"))?;
    ensure!(
        id > 0 && id.to_string() == value,
        "transcription has a non-canonical track id"
    );
    Ok(value.to_owned())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use backend_contracts::pipeline::{Producer, TranscriptionMode};

    use super::*;

    fn result(status: WorkerStatus, reason: Option<WorkerReason>) -> TranscriptionResult {
        TranscriptionResult {
            sc_track_id: "42".to_owned(),
            upload_generation: 3,
            attempt: 2,
            mode: TranscriptionMode::Align,
            status,
            reason,
            detail: None,
            producer: Producer {
                worker_id: "gpu-main".to_owned(),
                build: "test".to_owned(),
                models: BTreeMap::new(),
                sync_version: Some("s2.a.b.c".to_owned()),
            },
            sync_version: "s2.a.b.c".to_owned(),
            confidence: Some(0.9),
            placed_share: Some(0.975),
            aligned_share: Some(0.9),
            lines_total: Some(40),
            lines_unplaced: Some(1),
            language: Some("ru".to_owned()),
            synced_lrc: (status == WorkerStatus::Ok).then(|| "[00:01.00]line".to_owned()),
            words: None,
        }
    }

    #[test]
    fn every_status_lands_in_its_own_state() -> anyhow::Result<()> {
        let cases = [
            (WorkerStatus::Ok, None, Outcome::Aligned),
            (
                WorkerStatus::Empty,
                Some(WorkerReason::SilentAudio),
                Outcome::Empty,
            ),
            (
                WorkerStatus::Rejected,
                Some(WorkerReason::LowConfidence),
                Outcome::Rejected,
            ),
            (
                WorkerStatus::Missing,
                Some(WorkerReason::AudioNotFound),
                Outcome::Quarantined,
            ),
            (
                WorkerStatus::Failed,
                Some(WorkerReason::UndecodableAudio),
                Outcome::Quarantined,
            ),
            (
                WorkerStatus::Failed,
                Some(WorkerReason::DeadlineExceeded),
                Outcome::Quarantined,
            ),
            (
                WorkerStatus::Failed,
                Some(WorkerReason::EngineRestarted),
                Outcome::Reopenable,
            ),
            (
                WorkerStatus::Failed,
                Some(WorkerReason::PublicNodeTimeout),
                Outcome::Reopenable,
            ),
        ];
        for (status, reason, expected) in cases {
            assert_eq!(validate(result(status, reason))?.outcome, expected);
        }
        Ok(())
    }

    #[test]
    fn only_a_version_shaped_like_a_worker_build_counts_as_released() {
        for released in [
            "s3.1f3a9c2e.7d1b0e44.9a8b7c6d",
            "s2.a.b.c",
            "s10.main.v1-0.abc_1",
        ] {
            assert!(is_released_sync_version(released), "{released}");
        }
        for foreign in [
            "s1.old",
            "old",
            "emu-timeout",
            "s.a.b.c",
            "sx.a.b.c",
            "s3.a.b.c.d",
            "s3.a..c",
            "s3.123456789.b.c",
            "s3.a b.c.d",
            "",
        ] {
            assert!(!is_released_sync_version(foreign), "{foreign}");
        }
    }

    #[test]
    fn an_aligned_result_keeps_its_lyrics_and_metrics() -> anyhow::Result<()> {
        let validated = validate(result(WorkerStatus::Ok, None))?;

        assert_eq!(validated.synced_lrc.as_deref(), Some("[00:01.00]line"));
        assert_eq!(validated.lines_total, Some(40));
        assert_eq!(validated.lines_unplaced, Some(1));
        assert_eq!(validated.language.as_deref(), Some("ru"));
        Ok(())
    }

    #[test]
    fn a_reason_must_belong_to_its_status() {
        assert!(validate(result(WorkerStatus::Ok, Some(WorkerReason::LowConfidence))).is_err());
        assert!(validate(result(WorkerStatus::Rejected, None)).is_err());
        assert!(
            validate(result(
                WorkerStatus::Rejected,
                Some(WorkerReason::AudioNotFound)
            ))
            .is_err()
        );
    }

    #[test]
    fn an_aligned_result_without_lyrics_is_invalid() {
        let mut aligned = result(WorkerStatus::Ok, None);
        aligned.synced_lrc = Some("  ".to_owned());
        assert!(validate(aligned).is_err());
    }

    #[test]
    fn identity_and_shares_are_checked() {
        let mut urn = result(WorkerStatus::Ok, None);
        urn.sc_track_id = "soundcloud:tracks:42".to_owned();
        let mut zero_attempt = result(WorkerStatus::Ok, None);
        zero_attempt.attempt = 0;
        let mut share = result(WorkerStatus::Ok, None);
        share.placed_share = Some(1.5);
        let mut version = result(WorkerStatus::Ok, None);
        version.sync_version = " ".to_owned();

        assert!(validate(urn).is_err());
        assert!(validate(zero_attempt).is_err());
        assert!(validate(share).is_err());
        assert!(validate(version).is_err());
    }
}
