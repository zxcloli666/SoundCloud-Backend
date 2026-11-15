use std::time::Duration;

use backend_contracts::pipeline::HIGHLIGHT_VERSION;
use sqlx::PgPool;
use tracing::{debug, warn};
use uuid::Uuid;

use crate::queue::{JobError, JobResult};

#[derive(Debug, PartialEq, Eq)]
pub(super) struct Claimed {
    pub(super) sc_track_id: String,
    pub(super) attempt: i32,
    pub(super) version: i16,
}

impl Claimed {
    pub(super) fn message_id(&self) -> String {
        format!("highlight:{}:{}", self.sc_track_id, self.attempt)
    }
}

#[derive(Debug, PartialEq, Eq)]
struct Walked {
    last: Option<Uuid>,
    walked: i64,
    queued: i64,
}

pub(super) async fn walk(pool: &PgPool, after: Uuid, batch: i64) -> JobResult<Uuid> {
    let walked = sqlx::query_file_as!(
        Walked,
        "queries/highlights/walk.sql",
        after,
        batch,
        HIGHLIGHT_VERSION
    )
    .fetch_one(pool)
    .await
    .map_err(JobError::retryable)?;
    debug!(
        walked = walked.walked,
        queued = walked.queued,
        "highlight walk moved on"
    );
    Ok(match walked.last {
        Some(last) if walked.walked >= batch => last,
        _ => Uuid::nil(),
    })
}

pub(super) async fn claim(
    pool: &PgPool,
    in_flight: i64,
    timeout: Duration,
) -> JobResult<Vec<Claimed>> {
    let timeout_s = i64::try_from(timeout.as_secs()).unwrap_or(i64::MAX);
    sqlx::query_file!("queries/highlights/unanswered.sql", timeout_s)
        .execute(pool)
        .await
        .map_err(JobError::retryable)?;
    sqlx::query_file_as!(Claimed, "queries/highlights/claim.sql", in_flight)
        .fetch_all(pool)
        .await
        .map_err(JobError::retryable)
}

pub(super) async fn release(pool: &PgPool, claim: &Claimed) {
    let released = sqlx::query_file!(
        "queries/highlights/release.sql",
        &claim.sc_track_id,
        claim.attempt
    )
    .execute(pool)
    .await;
    if let Err(error) = released {
        warn!(
            track = %claim.sc_track_id,
            attempt = claim.attempt,
            %error,
            "unpublished highlight claim waits for the unanswered timeout"
        );
    }
}
