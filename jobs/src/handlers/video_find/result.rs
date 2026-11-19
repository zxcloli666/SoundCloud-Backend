use std::time::Duration;

use anyhow::{bail, ensure};
use backend_contracts::pipeline::{VideoClip, VideoFindResult, VideoKind, VideoPart};
use backend_contracts::reasons::{FailureClass, WorkerReason, WorkerStatus};
use backend_contracts::worker_contract::WorkerLane;
use serde_json::{Value, json};
use sqlx::PgPool;
use tracing::{debug, info, warn};

use super::stage::{Staged, StagedCandidate};
use super::ytdlp::Tube;
use super::{Pacing, VideoFind, queue};
use crate::handlers::video_accounts::Fetch;
use crate::objects::Blobs;
use crate::queue::{JobError, JobResult};

#[derive(Debug, PartialEq)]
pub(super) struct Found {
    pub(super) kind: VideoKind,
    pub(super) candidate: u32,
    pub(super) offset_ms: i32,
    pub(super) duration_ms: i32,
    pub(super) looped: bool,
    pub(super) scores: Value,
}

#[derive(Debug, PartialEq)]
pub(super) enum Verdict {
    Found(Box<Found>, (VideoClip, VideoPart)),
    Held(Box<Found>),
    Settle(WorkerReason),
    Retry(WorkerReason),
}

impl<F: Fetch + 'static, B: Blobs + 'static, T: Tube + 'static> VideoFind<F, B, T> {
    pub async fn finish(&self, result: VideoFindResult) -> JobResult {
        let attempt = i32::try_from(result.attempt)
            .map_err(|_| JobError::permanent(anyhow::anyhow!("video attempt is out of range")))?;
        let track = result.sc_track_id.as_str();
        match judge(&result).map_err(JobError::permanent)? {
            Verdict::Found(found, (clip, part)) => {
                let file = self
                    .parts
                    .add(track, result.attempt, &clip, &part)
                    .map_err(JobError::permanent)?;
                match file {
                    Some(file) => self.finish_clip(track, attempt, &found, file).await,
                    None => Ok(()),
                }
            }
            verdict => self.settle_staged(track, attempt, &verdict, &result).await,
        }
    }

    async fn settle_staged(
        &self,
        track: &str,
        attempt: i32,
        verdict: &Verdict,
        result: &VideoFindResult,
    ) -> JobResult {
        let Some(staged) = queue::staged(&self.pool, track, attempt).await? else {
            debug!(track, attempt, "stale video result ignored");
            return Ok(());
        };
        let held = self
            .settle_verdict(track, attempt, verdict, &staged, result)
            .await?;
        self.drop_temps(&leftover(&staged, held)).await;
        Ok(())
    }

    async fn finish_clip(
        &self,
        track: &str,
        attempt: i32,
        found: &Found,
        file: Vec<u8>,
    ) -> JobResult {
        let Some(staged) = queue::staged(&self.pool, track, attempt).await? else {
            debug!(track, attempt, "stale video result ignored");
            return Ok(());
        };
        let candidate = candidate_of(&staged, found.candidate)?;
        let key = final_key(track, attempt);
        self.blobs
            .put_bytes(&key, file)
            .await
            .map_err(JobError::retryable)?;
        if !store_found(&self.pool, track, attempt, found, candidate, &key).await? {
            let _ = self.blobs.delete(&key).await;
        }
        self.drop_temps(&staged.keys()).await;
        Ok(())
    }

    async fn settle_verdict(
        &self,
        track: &str,
        attempt: i32,
        verdict: &Verdict,
        staged: &Staged,
        result: &VideoFindResult,
    ) -> JobResult<Option<u32>> {
        let detail = result.detail.as_deref();
        match verdict {
            Verdict::Held(found) => {
                let candidate = candidate_of(staged, found.candidate)?;
                hold(&self.pool, track, attempt, found, candidate).await?;
                Ok(Some(found.candidate))
            }
            Verdict::Found(..) => Ok(None),
            Verdict::Settle(reason) => {
                let wait = match reason {
                    WorkerReason::StaticVideo => self.pacing.static_recheck,
                    _ => self.pacing.none_recheck,
                };
                settle(&self.pool, track, attempt, wait, *reason).await?;
                info!(
                    track,
                    attempt,
                    reason = reason.as_str(),
                    detail,
                    "video search settled"
                );
                Ok(None)
            }
            Verdict::Retry(reason) => {
                retry(&self.pool, &self.pacing, track, attempt, *reason).await?;
                warn!(
                    track,
                    attempt,
                    reason = reason.as_str(),
                    detail,
                    "video search failed"
                );
                Ok(None)
            }
        }
    }
}

fn leftover(staged: &Staged, held: Option<u32>) -> Vec<String> {
    staged
        .candidates
        .iter()
        .filter(|candidate| Some(candidate.index) != held)
        .map(|candidate| candidate.key.clone())
        .collect()
}

fn candidate_of(staged: &Staged, index: u32) -> JobResult<&StagedCandidate> {
    staged.candidate(index).ok_or_else(|| {
        JobError::permanent(anyhow::anyhow!(
            "video result names candidate {index} it was never sent"
        ))
    })
}

pub(crate) fn final_key(track: &str, attempt: i32) -> String {
    format!("video/{track}-{attempt}.mp4")
}

async fn store_found(
    pool: &PgPool,
    track: &str,
    attempt: i32,
    found: &Found,
    candidate: &StagedCandidate,
    key: &str,
) -> JobResult<bool> {
    let mut transaction = pool.begin().await.map_err(JobError::retryable)?;
    let settled = sqlx::query_file!(
        "queries/video_find/settle.sql",
        track,
        attempt,
        "found",
        0.0_f64,
        None::<&str>,
        None::<Value>
    )
    .fetch_optional(&mut *transaction)
    .await
    .map_err(JobError::retryable)?;
    if settled.is_none() {
        debug!(track, attempt, "stale video result ignored");
        return Ok(false);
    }
    sqlx::query_file!(
        "queries/video_find/store_video.sql",
        track,
        found.kind.as_str(),
        candidate.source.as_str(),
        candidate.source_ref,
        key,
        found.offset_ms,
        found.looped,
        found.duration_ms,
        found.scores
    )
    .execute(&mut *transaction)
    .await
    .map_err(JobError::retryable)?;
    transaction.commit().await.map_err(JobError::retryable)?;
    info!(track, attempt, kind = found.kind.as_str(), "video stored");
    Ok(true)
}

async fn hold(
    pool: &PgPool,
    track: &str,
    attempt: i32,
    found: &Found,
    candidate: &StagedCandidate,
) -> JobResult {
    let payload = json!({
        "kind": found.kind.as_str(),
        "s3_key": candidate.key,
        "offset_ms": found.offset_ms,
        "duration_ms": found.duration_ms,
        "loop": found.looped,
        "source": candidate.source.as_str(),
        "source_ref": candidate.source_ref,
        "scores": found.scores,
    });
    sqlx::query_file!(
        "queries/video_find/settle.sql",
        track,
        attempt,
        "pending_vlm",
        0.0_f64,
        None::<&str>,
        payload
    )
    .fetch_optional(pool)
    .await
    .map_err(JobError::retryable)?;
    info!(track, attempt, "uncertain video waits for the VLM check");
    Ok(())
}

pub(super) async fn settle(
    pool: &PgPool,
    track: &str,
    attempt: i32,
    wait: Duration,
    reason: WorkerReason,
) -> JobResult {
    let stage = if reason == WorkerReason::StaticVideo {
        "static"
    } else {
        "none"
    };
    sqlx::query_file!(
        "queries/video_find/settle.sql",
        track,
        attempt,
        stage,
        wait.as_secs_f64(),
        reason.as_str(),
        None::<Value>
    )
    .fetch_optional(pool)
    .await
    .map_err(JobError::retryable)?;
    Ok(())
}

async fn retry(
    pool: &PgPool,
    pacing: &Pacing,
    track: &str,
    attempt: i32,
    reason: WorkerReason,
) -> JobResult {
    sqlx::query_file!(
        "queries/video_find/retry.sql",
        track,
        attempt,
        pacing.retry_base.as_secs_f64(),
        i32::try_from(pacing.max_attempts).unwrap_or(i32::MAX),
        reason.as_str(),
        pacing.none_recheck.as_secs_f64()
    )
    .fetch_optional(pool)
    .await
    .map_err(JobError::retryable)?;
    Ok(())
}

pub(super) fn judge(result: &VideoFindResult) -> anyhow::Result<Verdict> {
    ensure!(
        catalog_ingest::normalize_sc_track_id(&result.sc_track_id).as_deref()
            == Some(result.sc_track_id.as_str()),
        "video result has a non-canonical track id"
    );
    match (result.status, result.reason) {
        (WorkerStatus::Ok, None) => found(result),
        (WorkerStatus::Ok, Some(reason)) => {
            bail!("video ok result carries reason {}", reason.as_str())
        }
        (status, None) => bail!("video {} result has no reason", status.as_str()),
        (status, Some(reason)) => {
            ensure!(
                reason.status() == status && reason.is_published_on(WorkerLane::VideoFind),
                "video result carries reason {} its lane never publishes as {}",
                reason.as_str(),
                status.as_str()
            );
            Ok(match reason.failure_class() {
                Some(FailureClass::Transient | FailureClass::Reopenable) => Verdict::Retry(reason),
                _ if status == WorkerStatus::Missing => Verdict::Retry(reason),
                _ => Verdict::Settle(reason),
            })
        }
    }
}

fn found(result: &VideoFindResult) -> anyhow::Result<Verdict> {
    let (Some(kind), Some(candidate), Some(offset_ms), Some(duration_ms)) = (
        result.kind,
        result.candidate,
        result.offset_ms,
        result.duration_ms,
    ) else {
        bail!("video ok result lacks its verdict");
    };
    ensure!(
        offset_ms >= 0 && duration_ms > 0,
        "video window {offset_ms}+{duration_ms} is invalid"
    );
    let file = match (&result.clip, &result.part) {
        (Some(clip), Some(part)) => Some((clip.clone(), part.clone())),
        (None, None) => None,
        _ => bail!("video result carries half a file"),
    };
    let window = Box::new(Found {
        kind,
        candidate,
        offset_ms,
        duration_ms,
        looped: result.looped.unwrap_or(kind == VideoKind::Loop),
        scores: serde_json::to_value(result.scores.clone().unwrap_or_default())?,
    });
    if result.uncertain.unwrap_or(false) {
        return Ok(Verdict::Held(window));
    }
    Ok(match file {
        Some(file) => Verdict::Found(window, file),
        None if kind == VideoKind::Loop => Verdict::Retry(WorkerReason::InternalError),
        None => bail!("video clip result carries no file"),
    })
}
