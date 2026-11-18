use std::time::Duration;

use anyhow::{bail, ensure};
use backend_contracts::pipeline::{VideoFindResult, VideoKind};
use backend_contracts::reasons::{FailureClass, WorkerReason, WorkerStatus};
use backend_contracts::worker_contract::WorkerLane;
use serde_json::{Value, json};
use sqlx::PgPool;
use tracing::{debug, info, warn};

use super::Pacing;
use crate::handlers::video_accounts::store;
use crate::queue::{JobError, JobResult};

#[derive(Debug, PartialEq)]
pub(super) struct Clip {
    pub(super) kind: VideoKind,
    pub(super) s3_key: String,
    pub(super) offset_ms: i32,
    pub(super) duration_ms: i32,
    pub(super) looped: bool,
    pub(super) source: String,
    pub(super) source_ref: Option<String>,
    pub(super) scores: Value,
    pub(super) uncertain: bool,
}

#[derive(Debug, PartialEq)]
pub(super) enum Verdict {
    Found(Box<Clip>),
    Settle(WorkerReason),
    Retry(WorkerReason),
}

pub(super) async fn record(pool: &PgPool, pacing: &Pacing, result: &VideoFindResult) -> JobResult {
    let attempt = i32::try_from(result.attempt)
        .map_err(|_| JobError::permanent(anyhow::anyhow!("video attempt is out of range")))?;
    let track = result.sc_track_id.as_str();
    let detail = result.detail.as_deref();
    match judge(result).map_err(JobError::permanent)? {
        Verdict::Found(clip) if clip.uncertain => hold(pool, track, attempt, &clip).await?,
        Verdict::Found(clip) => store_clip(pool, track, attempt, &clip).await?,
        Verdict::Settle(reason) => {
            let wait = match reason {
                WorkerReason::StaticVideo => pacing.static_recheck,
                _ => pacing.none_recheck,
            };
            settle(pool, track, attempt, wait, reason).await?;
            info!(
                track,
                attempt,
                reason = reason.as_str(),
                detail,
                "video search settled"
            );
        }
        Verdict::Retry(reason) => {
            account_effect(pool, pacing, result, reason).await?;
            retry(pool, pacing, track, attempt, reason).await?;
            warn!(
                track,
                attempt,
                reason = reason.as_str(),
                detail,
                "video search failed"
            );
        }
    }
    Ok(())
}

async fn store_clip(pool: &PgPool, track: &str, attempt: i32, clip: &Clip) -> JobResult {
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
        return Ok(());
    }
    sqlx::query_file!(
        "queries/video_find/store_video.sql",
        track,
        kind_name(clip.kind),
        clip.source,
        clip.source_ref,
        clip.s3_key,
        clip.offset_ms,
        clip.looped,
        clip.duration_ms,
        clip.scores
    )
    .execute(&mut *transaction)
    .await
    .map_err(JobError::retryable)?;
    transaction.commit().await.map_err(JobError::retryable)?;
    info!(track, attempt, kind = kind_name(clip.kind), "video stored");
    Ok(())
}

async fn hold(pool: &PgPool, track: &str, attempt: i32, clip: &Clip) -> JobResult {
    let payload = json!({
        "kind": kind_name(clip.kind),
        "s3_key": clip.s3_key,
        "offset_ms": clip.offset_ms,
        "duration_ms": clip.duration_ms,
        "loop": clip.looped,
        "source": clip.source,
        "source_ref": clip.source_ref,
        "scores": clip.scores,
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

async fn settle(
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
    let uncounted = reason == WorkerReason::SourceRateLimited;
    sqlx::query_file!(
        "queries/video_find/retry.sql",
        track,
        attempt,
        pacing.retry_base.as_secs_f64(),
        i32::try_from(pacing.max_attempts).unwrap_or(i32::MAX),
        reason.as_str(),
        uncounted,
        pacing.none_recheck.as_secs_f64()
    )
    .fetch_optional(pool)
    .await
    .map_err(JobError::retryable)?;
    Ok(())
}

async fn account_effect(
    pool: &PgPool,
    pacing: &Pacing,
    result: &VideoFindResult,
    reason: WorkerReason,
) -> JobResult {
    let account = result.account.as_str();
    let outcome = match reason {
        WorkerReason::SourceRateLimited => {
            store::pause(pool, account, pacing.account_pause, reason.as_str()).await
        }
        WorkerReason::SourceAuthFailed => store::kill(pool, account, reason.as_str()).await,
        _ => return Ok(()),
    };
    outcome.map_err(JobError::retryable)?;
    warn!(
        account,
        reason = reason.as_str(),
        "video account taken out of rotation"
    );
    Ok(())
}

pub(super) fn judge(result: &VideoFindResult) -> anyhow::Result<Verdict> {
    ensure!(
        catalog_ingest::normalize_sc_track_id(&result.sc_track_id).as_deref()
            == Some(result.sc_track_id.as_str()),
        "video result has a non-canonical track id"
    );
    match (result.status, result.reason) {
        (WorkerStatus::Ok, None) => Ok(Verdict::Found(Box::new(clip(result)?))),
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

fn clip(result: &VideoFindResult) -> anyhow::Result<Clip> {
    let (Some(kind), Some(s3_key), Some(offset_ms), Some(duration_ms), Some(source)) = (
        result.kind,
        result.s3_key.clone(),
        result.offset_ms,
        result.duration_ms,
        result.source,
    ) else {
        bail!("video ok result lacks its file");
    };
    ensure!(
        offset_ms >= 0 && duration_ms > 0,
        "video window {offset_ms}+{duration_ms} is invalid"
    );
    ensure!(
        s3_key.starts_with("video/") && !s3_key.contains(".."),
        "video key {s3_key} is outside the video folder"
    );
    let scores = serde_json::to_value(result.scores.clone().unwrap_or_default())?;
    Ok(Clip {
        kind,
        s3_key,
        offset_ms,
        duration_ms,
        looped: result.looped.unwrap_or(kind == VideoKind::Loop),
        source: source.as_str().to_owned(),
        source_ref: result.source_ref.clone(),
        scores,
        uncertain: result.uncertain.unwrap_or(false),
    })
}

fn kind_name(kind: VideoKind) -> &'static str {
    kind.as_str()
}
