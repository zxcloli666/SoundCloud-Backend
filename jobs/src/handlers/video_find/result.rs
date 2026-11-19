use anyhow::{bail, ensure};
use backend_contracts::pipeline::{
    MAX_VIDEO_NEXT, VideoClip, VideoFindResult, VideoKind, VideoNext, VideoPart,
};
use backend_contracts::reasons::{FailureClass, WorkerReason, WorkerStatus};
use backend_contracts::worker_contract::WorkerLane;
use serde_json::{Value, json};
use sqlx::PgPool;
use tracing::{debug, info, warn};

use super::stage::{Staged, StagedCandidate, cut_key};
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
    pub(super) next: Vec<VideoNext>,
}

#[derive(Debug, PartialEq)]
pub(super) enum Verdict {
    Found(Box<Found>, (VideoClip, VideoPart)),
    Bare(Box<Found>),
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
                    Some(file) => self.finish_cut(track, attempt, &found, file).await,
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
        self.drop_temps(&leftover(&staged, &held)).await;
        Ok(())
    }

    async fn finish_cut(
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
        if candidate.kept {
            keep(&self.pool, &self.pacing, track, attempt).await?;
            self.drop_temps(&staged.keys()).await;
            return Ok(());
        }
        let key = cut_key(track, attempt);
        self.blobs
            .put_bytes(&key, file)
            .await
            .map_err(JobError::retryable)?;
        let held = hold(
            &self.pool,
            &self.pacing,
            track,
            attempt,
            found,
            &staged,
            &key,
        )
        .await?;
        let Some(held) = held else {
            self.drop_temps(std::slice::from_ref(&key)).await;
            return Ok(());
        };
        self.drop_temps(&leftover(&staged, &held)).await;
        Ok(())
    }

    async fn settle_verdict(
        &self,
        track: &str,
        attempt: i32,
        verdict: &Verdict,
        staged: &Staged,
        result: &VideoFindResult,
    ) -> JobResult<Vec<u32>> {
        let detail = result.detail.as_deref();
        match verdict {
            Verdict::Bare(found) if candidate_of(staged, found.candidate)?.kept => {
                keep(&self.pool, &self.pacing, track, attempt).await?;
                Ok(Vec::new())
            }
            Verdict::Bare(found) => {
                let key = &candidate_of(staged, found.candidate)?.key;
                let held =
                    hold(&self.pool, &self.pacing, track, attempt, found, staged, key).await?;
                let mut held = held.unwrap_or_default();
                held.push(found.candidate);
                Ok(held)
            }
            Verdict::Found(..) => Ok(Vec::new()),
            Verdict::Settle(reason) => {
                settle(&self.pool, &self.pacing, track, attempt, *reason).await?;
                info!(
                    track,
                    attempt,
                    reason = reason.as_str(),
                    detail,
                    "video search settled"
                );
                Ok(Vec::new())
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
                Ok(Vec::new())
            }
        }
    }
}

fn leftover(staged: &Staged, held: &[u32]) -> Vec<String> {
    staged
        .candidates
        .iter()
        .filter(|candidate| !candidate.kept && !held.contains(&candidate.index))
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

async fn keep(pool: &PgPool, pacing: &Pacing, track: &str, attempt: i32) -> JobResult {
    let kept = sqlx::query_file!(
        "queries/video_find/settle.sql",
        track,
        attempt,
        "found",
        0.0_f64,
        None::<&str>,
        None::<Value>,
        pacing.research_fresh.as_secs_f64(),
        pacing.research_old.as_secs_f64(),
        pacing.research_fresh_age.as_secs_f64()
    )
    .fetch_optional(pool)
    .await
    .map_err(JobError::retryable)?;
    if kept.is_some() {
        info!(track, attempt, "video search kept the stored video");
    }
    Ok(())
}

async fn hold(
    pool: &PgPool,
    pacing: &Pacing,
    track: &str,
    attempt: i32,
    found: &Found,
    staged: &Staged,
    key: &str,
) -> JobResult<Option<Vec<u32>>> {
    let candidate = candidate_of(staged, found.candidate)?;
    let cut = key != candidate.key;
    let next: Vec<(u32, Value)> = found
        .next
        .iter()
        .filter_map(|next| {
            let candidate = staged.candidate(next.candidate).filter(|one| !one.kept)?;
            Some((next.candidate, held_next(next, candidate)))
        })
        .take(MAX_VIDEO_NEXT as usize)
        .collect();
    let payload = json!({
        "kind": found.kind.as_str(),
        "s3_key": key,
        "file": if cut { "cut" } else { "raw" },
        "offset_ms": found.offset_ms,
        "duration_ms": found.duration_ms,
        "loop": found.looped,
        "source": candidate.source.as_str(),
        "source_ref": candidate.source_ref,
        "scores": found.scores,
        "next": next.iter().map(|(_, next)| next).collect::<Vec<_>>(),
    });
    let held = sqlx::query_file!(
        "queries/video_find/settle.sql",
        track,
        attempt,
        "pending_vlm",
        0.0_f64,
        None::<&str>,
        payload,
        pacing.research_fresh.as_secs_f64(),
        pacing.research_old.as_secs_f64(),
        pacing.research_fresh_age.as_secs_f64()
    )
    .fetch_optional(pool)
    .await
    .map_err(JobError::retryable)?;
    if held.is_none() {
        return Ok(None);
    }
    info!(
        track,
        attempt,
        kind = found.kind.as_str(),
        cut,
        next = next.len(),
        "video waits for the VLM check"
    );
    Ok(Some(next.into_iter().map(|(index, _)| index).collect()))
}

fn held_next(next: &VideoNext, candidate: &StagedCandidate) -> Value {
    json!({
        "kind": next.kind.as_str(),
        "s3_key": candidate.key,
        "offset_ms": next.offset_ms,
        "pad_ms": next.pad_ms,
        "duration_ms": next.duration_ms,
        "loop": next.looped,
        "source": candidate.source.as_str(),
        "source_ref": candidate.source_ref,
        "scores": next.scores,
    })
}

pub(super) async fn settle(
    pool: &PgPool,
    pacing: &Pacing,
    track: &str,
    attempt: i32,
    reason: WorkerReason,
) -> JobResult {
    let (stage, wait) = if reason == WorkerReason::StaticVideo {
        ("static", pacing.static_recheck)
    } else {
        ("none", pacing.none_recheck)
    };
    sqlx::query_file!(
        "queries/video_find/settle.sql",
        track,
        attempt,
        stage,
        wait.as_secs_f64(),
        reason.as_str(),
        None::<Value>,
        pacing.research_fresh.as_secs_f64(),
        pacing.research_old.as_secs_f64(),
        pacing.research_fresh_age.as_secs_f64()
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
        next: result.next.clone().unwrap_or_default(),
    });
    Ok(match file {
        Some(file) => Verdict::Found(window, file),
        None => Verdict::Bare(window),
    })
}
