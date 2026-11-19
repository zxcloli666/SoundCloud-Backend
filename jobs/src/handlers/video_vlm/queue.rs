use std::time::Duration;

use backend_contracts::admin_video::VideoSource;
use serde::Deserialize;
use serde_json::Value;
use sqlx::PgPool;
use tracing::warn;

use crate::handlers::video_find::Pacing;
use crate::queue::{JobError, JobResult};

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub(super) struct Held {
    pub(super) s3_key: String,
    pub(super) offset_ms: i32,
    pub(super) duration_ms: i32,
    pub(super) source: VideoSource,
    pub(super) source_ref: Option<String>,
    pub(super) scores: Value,
    #[serde(default)]
    pub(super) vlm_sent: i64,
}

#[derive(Debug, PartialEq)]
pub(super) struct Claimed {
    pub(super) sc_track_id: String,
    pub(super) attempt: i32,
    pub(super) held: Option<Held>,
    pub(super) duration_ms: Option<i32>,
}

impl Claimed {
    pub(super) fn message_id(&self, held: &Held) -> String {
        format!(
            "video_vlm:{}:{}:{}",
            self.sc_track_id, self.attempt, held.vlm_sent
        )
    }
}

pub(super) async fn requeue_unanswered(pool: &PgPool, timeout: Duration) -> JobResult {
    sqlx::query_file!("queries/video_vlm/unanswered.sql", timeout.as_secs_f64())
        .execute(pool)
        .await
        .map_err(JobError::retryable)?;
    Ok(())
}

pub(super) async fn busy(pool: &PgPool) -> JobResult<i64> {
    let row = sqlx::query_file!("queries/video_vlm/busy.sql")
        .fetch_one(pool)
        .await
        .map_err(JobError::retryable)?;
    Ok(row.busy)
}

pub(super) async fn claim(pool: &PgPool, limit: i64) -> JobResult<Vec<Claimed>> {
    let rows = sqlx::query_file!("queries/video_vlm/claim.sql", limit)
        .fetch_all(pool)
        .await
        .map_err(JobError::retryable)?;
    Ok(rows
        .into_iter()
        .map(|row| Claimed {
            sc_track_id: row.sc_track_id,
            attempt: row.attempt,
            held: serde_json::from_value(row.payload).ok(),
            duration_ms: row.duration_ms,
        })
        .collect())
}

pub(super) async fn held(pool: &PgPool, track: &str, attempt: i32) -> JobResult<Option<Held>> {
    let row = sqlx::query_file!("queries/video_vlm/held.sql", track, attempt)
        .fetch_optional(pool)
        .await
        .map_err(JobError::retryable)?;
    Ok(row
        .and_then(|row| row.payload)
        .and_then(|payload| serde_json::from_value(payload).ok()))
}

pub(super) async fn release(pool: &PgPool, track: &str, attempt: i32, wait: Duration, why: &str) {
    let released = sqlx::query_file!(
        "queries/video_vlm/release.sql",
        track,
        attempt,
        wait.as_secs_f64(),
        why
    )
    .execute(pool)
    .await;
    if let Err(error) = released {
        warn!(
            track,
            attempt,
            %error,
            "unsent VLM claim waits for the unanswered timeout"
        );
    }
}

pub(super) async fn settle(
    pool: &PgPool,
    track: &str,
    attempt: i32,
    stage: &str,
    wait: Duration,
    why: Option<&str>,
) -> JobResult<bool> {
    let settled = sqlx::query_file!(
        "queries/video_vlm/settle.sql",
        track,
        attempt,
        stage,
        wait.as_secs_f64(),
        why
    )
    .fetch_optional(pool)
    .await
    .map_err(JobError::retryable)?;
    Ok(settled.is_some())
}

pub(super) async fn again(
    pool: &PgPool,
    track: &str,
    attempt: i32,
    why: &str,
    pacing: &Pacing,
) -> JobResult<Option<String>> {
    let row = sqlx::query_file!(
        "queries/video_vlm/again.sql",
        track,
        attempt,
        pacing.retry_base.as_secs_f64(),
        i32::try_from(pacing.max_attempts).unwrap_or(i32::MAX),
        why,
        pacing.none_recheck.as_secs_f64()
    )
    .fetch_optional(pool)
    .await
    .map_err(JobError::retryable)?;
    Ok(row.map(|row| row.stage))
}

pub(super) async fn rebuild(
    pool: &PgPool,
    track: &str,
    attempt: i32,
    why: &str,
    pacing: &Pacing,
) -> JobResult<Option<String>> {
    let row = sqlx::query_file!(
        "queries/video_vlm/rebuild.sql",
        track,
        attempt,
        pacing.retry_base.as_secs_f64(),
        i32::try_from(pacing.max_attempts).unwrap_or(i32::MAX),
        why,
        pacing.none_recheck.as_secs_f64()
    )
    .fetch_optional(pool)
    .await
    .map_err(JobError::retryable)?;
    Ok(row.map(|row| row.stage))
}
