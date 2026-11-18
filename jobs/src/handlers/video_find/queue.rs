use std::time::Duration;

use sqlx::PgPool;
use tracing::warn;

use super::stage::Staged;
use crate::queue::{JobError, JobResult};

#[derive(Debug, PartialEq, Eq)]
pub(super) struct Claimed {
    pub(super) sc_track_id: String,
    pub(super) attempt: i32,
    pub(super) title: String,
    pub(super) artist: String,
    pub(super) isrc: Option<String>,
    pub(super) duration_ms: Option<i32>,
}

impl Claimed {
    pub(super) fn message_id(&self) -> String {
        format!("video_find:{}:{}", self.sc_track_id, self.attempt)
    }
}

pub(super) async fn requeue_unanswered(pool: &PgPool, timeout: Duration) -> JobResult {
    sqlx::query_file!("queries/video_find/unanswered.sql", timeout.as_secs_f64())
        .execute(pool)
        .await
        .map_err(JobError::retryable)?;
    Ok(())
}

pub(super) async fn busy(pool: &PgPool) -> JobResult<i64> {
    let row = sqlx::query_file!("queries/video_find/busy.sql")
        .fetch_one(pool)
        .await
        .map_err(JobError::retryable)?;
    Ok(row.busy)
}

pub(super) async fn claim(pool: &PgPool, limit: i64) -> JobResult<Vec<Claimed>> {
    sqlx::query_file_as!(Claimed, "queries/video_find/claim.sql", limit)
        .fetch_all(pool)
        .await
        .map_err(JobError::retryable)
}

pub(super) async fn stage(pool: &PgPool, claim: &Claimed, staged: &Staged) -> JobResult<bool> {
    let payload = serde_json::to_value(staged).map_err(JobError::permanent)?;
    let marked = sqlx::query_file!(
        "queries/video_find/stage.sql",
        &claim.sc_track_id,
        claim.attempt,
        payload
    )
    .execute(pool)
    .await
    .map_err(JobError::retryable)?;
    Ok(marked.rows_affected() == 1)
}

pub(super) async fn staged(pool: &PgPool, track: &str, attempt: i32) -> JobResult<Option<Staged>> {
    let row = sqlx::query_file!("queries/video_find/staged.sql", track, attempt)
        .fetch_optional(pool)
        .await
        .map_err(JobError::retryable)?;
    Ok(row
        .and_then(|row| row.payload)
        .and_then(|payload| serde_json::from_value(payload).ok()))
}

pub(super) async fn release(pool: &PgPool, claim: &Claimed, wait: Duration, why: &str) {
    let released = sqlx::query_file!(
        "queries/video_find/release.sql",
        &claim.sc_track_id,
        claim.attempt,
        wait.as_secs_f64(),
        why
    )
    .execute(pool)
    .await;
    if let Err(error) = released {
        warn!(
            track = %claim.sc_track_id,
            attempt = claim.attempt,
            %error,
            "unpublished video claim waits for the unanswered timeout"
        );
    }
}
