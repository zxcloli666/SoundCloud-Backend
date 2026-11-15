use anyhow::{bail, ensure};
use backend_contracts::pipeline::HighlightResult;
use backend_contracts::reasons::{FailureClass, WorkerReason, WorkerStatus};
use backend_contracts::worker_contract::WorkerLane;
use sqlx::PgPool;
use tracing::{debug, info, warn};

use crate::queue::{JobError, JobResult};

#[derive(Debug, PartialEq)]
enum Verdict {
    Found(Moment),
    Retry(WorkerReason),
    Settle(WorkerReason),
}

#[derive(Debug, PartialEq)]
struct Moment {
    version: i16,
    start_ms: i32,
    length_ms: i32,
    score: f32,
}

pub(super) async fn record(
    pool: &PgPool,
    max_failures: u32,
    result: &HighlightResult,
) -> JobResult {
    let attempt = i32::try_from(result.attempt)
        .map_err(|_| JobError::permanent(anyhow::anyhow!("highlight attempt is out of range")))?;
    let track = result.sc_track_id.as_str();
    let detail = result.detail.as_deref();
    match judge(result).map_err(JobError::permanent)? {
        Verdict::Found(moment) => {
            let applied = sqlx::query_file!(
                "queries/highlights/apply.sql",
                track,
                attempt,
                moment.start_ms,
                moment.length_ms,
                moment.score,
                moment.version
            )
            .fetch_one(pool)
            .await
            .map_err(JobError::retryable)?;
            debug!(
                track,
                attempt,
                start_ms = moment.start_ms,
                score = moment.score,
                stored = applied.stored,
                settled = applied.settled,
                "highlight stored"
            );
        }
        Verdict::Retry(reason) => {
            sqlx::query_file!(
                "queries/highlights/retry.sql",
                track,
                attempt,
                reason.as_str(),
                i32::try_from(max_failures).unwrap_or(i32::MAX)
            )
            .execute(pool)
            .await
            .map_err(JobError::retryable)?;
            warn!(
                track,
                attempt,
                reason = reason.as_str(),
                detail,
                "highlight failed and backs off"
            );
        }
        Verdict::Settle(reason) => {
            sqlx::query_file!(
                "queries/highlights/settle.sql",
                track,
                attempt,
                reason.as_str()
            )
            .execute(pool)
            .await
            .map_err(JobError::retryable)?;
            info!(
                track,
                attempt,
                reason = reason.as_str(),
                detail,
                "highlight settled without a moment"
            );
        }
    }
    Ok(())
}

fn judge(result: &HighlightResult) -> anyhow::Result<Verdict> {
    ensure!(
        catalog_ingest::normalize_sc_track_id(&result.sc_track_id).as_deref()
            == Some(result.sc_track_id.as_str()),
        "highlight result has a non-canonical track id"
    );
    match (result.status, result.reason) {
        (WorkerStatus::Ok, None) => Ok(Verdict::Found(moment(result)?)),
        (WorkerStatus::Ok, Some(reason)) => {
            bail!("highlight ok result carries reason {}", reason.as_str())
        }
        (status, None) => bail!("highlight {} result has no reason", status.as_str()),
        (status, Some(reason)) => {
            ensure!(
                reason.status() == status && reason.is_published_on(WorkerLane::Highlight),
                "highlight result carries reason {} its lane never publishes as {}",
                reason.as_str(),
                status.as_str()
            );
            Ok(match reason.failure_class() {
                Some(FailureClass::Transient | FailureClass::Reopenable) => Verdict::Retry(reason),
                _ => Verdict::Settle(reason),
            })
        }
    }
}

fn moment(result: &HighlightResult) -> anyhow::Result<Moment> {
    let (Some(version), Some(start_ms), Some(length_ms), Some(score)) = (
        result.version,
        result.start_ms,
        result.length_ms,
        result.score,
    ) else {
        bail!("highlight ok result lacks its window");
    };
    ensure!(version > 0, "highlight version must be positive");
    ensure!(
        start_ms >= 0 && length_ms > 0,
        "highlight window {start_ms}+{length_ms} is invalid"
    );
    ensure!(
        score.is_finite() && (0.0..=1.0).contains(&score),
        "highlight score {score} is outside 0..1"
    );
    Ok(Moment {
        version,
        start_ms,
        length_ms,
        score,
    })
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use backend_contracts::pipeline::Producer;

    use super::*;

    fn result(status: WorkerStatus, reason: Option<WorkerReason>) -> HighlightResult {
        HighlightResult {
            sc_track_id: "98765".to_owned(),
            attempt: 2,
            status,
            reason,
            detail: None,
            producer: Producer {
                worker_id: "sc-worker1".to_owned(),
                build: "test".to_owned(),
                models: BTreeMap::new(),
                sync_version: None,
            },
            version: None,
            start_ms: None,
            length_ms: None,
            score: None,
        }
    }

    fn found() -> HighlightResult {
        HighlightResult {
            version: Some(1),
            start_ms: Some(52_500),
            length_ms: Some(30_000),
            score: Some(0.8),
            ..result(WorkerStatus::Ok, None)
        }
    }

    #[test]
    fn an_ok_result_is_a_moment() -> anyhow::Result<()> {
        assert_eq!(
            judge(&found())?,
            Verdict::Found(Moment {
                version: 1,
                start_ms: 52_500,
                length_ms: 30_000,
                score: 0.8
            })
        );
        Ok(())
    }

    #[test]
    fn an_ok_result_without_a_full_window_or_with_a_wild_score_is_refused() {
        let partial = HighlightResult {
            score: None,
            ..found()
        };
        let wild = HighlightResult {
            score: Some(1.5),
            ..found()
        };
        let negative = HighlightResult {
            start_ms: Some(-1),
            ..found()
        };

        assert!(judge(&partial).is_err());
        assert!(judge(&wild).is_err());
        assert!(judge(&negative).is_err());
    }

    #[test]
    fn transient_and_lost_work_retries_and_the_rest_settles() -> anyhow::Result<()> {
        let failed = |reason| judge(&result(WorkerStatus::Failed, Some(reason)));

        assert_eq!(
            failed(WorkerReason::DownloadFailed)?,
            Verdict::Retry(WorkerReason::DownloadFailed)
        );
        assert_eq!(
            failed(WorkerReason::WorkerLost)?,
            Verdict::Retry(WorkerReason::WorkerLost)
        );
        assert_eq!(
            failed(WorkerReason::UndecodableAudio)?,
            Verdict::Settle(WorkerReason::UndecodableAudio)
        );
        assert_eq!(
            judge(&result(
                WorkerStatus::Missing,
                Some(WorkerReason::AudioNotFound)
            ))?,
            Verdict::Settle(WorkerReason::AudioNotFound)
        );
        assert_eq!(
            judge(&result(
                WorkerStatus::Empty,
                Some(WorkerReason::SilentAudio)
            ))?,
            Verdict::Settle(WorkerReason::SilentAudio)
        );
        Ok(())
    }

    #[test]
    fn a_reason_the_lane_never_publishes_is_refused() {
        assert!(
            judge(&result(
                WorkerStatus::Failed,
                Some(WorkerReason::PublicNodeTimeout)
            ))
            .is_err()
        );
        assert!(
            judge(&result(
                WorkerStatus::Failed,
                Some(WorkerReason::DownloadFailed)
            ))
            .is_ok()
        );
        assert!(
            judge(&result(
                WorkerStatus::Empty,
                Some(WorkerReason::AudioTooShort)
            ))
            .is_err()
        );
        assert!(judge(&result(WorkerStatus::Failed, None)).is_err());
    }
}
