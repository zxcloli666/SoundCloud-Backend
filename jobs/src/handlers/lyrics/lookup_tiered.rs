use chrono::{DateTime, Utc};
use sqlx::PgPool;

use super::LookupClaim;
use crate::handlers::star_priority::{LIKED_PRIORITY, Pass, Round, STAR_SCAN_WINDOW, Turn};
use crate::queue::LeasedJob;

pub(super) async fn claim(
    pool: &PgPool,
    job: &LeasedJob,
    total: i64,
    claim_seconds: i64,
    turn: Turn,
) -> Result<Vec<LookupClaim>, sqlx::Error> {
    let oldest = (total / 4).max(1).min(total);
    let tiered = total - oldest;
    let mut round = Round::new("lyrics_lookup");
    let mut claims = Vec::new();
    for tier in turn.tiers() {
        let left = tiered.saturating_sub(i64::try_from(claims.len()).unwrap_or(i64::MAX));
        let claimed = if left > 0 {
            claim_pass(pool, job, tier.pass(), left, claim_seconds).await?
        } else {
            Vec::new()
        };
        let drained = i64::try_from(claimed.len()).unwrap_or(i64::MAX) < left;
        let next = if drained {
            None
        } else {
            next_waiting(pool, tier.pass()).await?
        };
        round.record(tier.label(), claimed.len(), next);
        claims.extend(claimed);
    }
    let oldest_claims = sqlx::query_file_as!(
        LookupClaim,
        "queries/lyrics/lookup_claim_oldest.sql",
        job.id,
        job.generation,
        job.lease_id,
        oldest,
        claim_seconds
    )
    .fetch_all(pool)
    .await?;
    let next_oldest = sqlx::query_file_scalar!("queries/lyrics/lookup_next_oldest.sql")
        .fetch_optional(pool)
        .await?;
    round.record("oldest", oldest_claims.len(), next_oldest);
    round.report();
    claims.extend(oldest_claims);
    Ok(claims)
}

async fn claim_pass(
    pool: &PgPool,
    job: &LeasedJob,
    pass: Pass,
    left: i64,
    claim_seconds: i64,
) -> Result<Vec<LookupClaim>, sqlx::Error> {
    match pass {
        Pass::Star(stored) => {
            sqlx::query_file_as!(
                LookupClaim,
                "queries/lyrics/lookup_claim_star.sql",
                job.id,
                job.generation,
                job.lease_id,
                left,
                claim_seconds,
                stored,
                STAR_SCAN_WINDOW
            )
            .fetch_all(pool)
            .await
        }
        Pass::Liked => {
            claim_by_priority(
                pool,
                job,
                left,
                claim_seconds,
                LIKED_PRIORITY,
                LIKED_PRIORITY,
            )
            .await
        }
        Pass::Rest => claim_by_priority(pool, job, left, claim_seconds, 0, i16::MAX).await,
    }
}

async fn claim_by_priority(
    pool: &PgPool,
    job: &LeasedJob,
    left: i64,
    claim_seconds: i64,
    lowest: i16,
    highest: i16,
) -> Result<Vec<LookupClaim>, sqlx::Error> {
    sqlx::query_file_as!(
        LookupClaim,
        "queries/lyrics/lookup_claim_priority.sql",
        job.id,
        job.generation,
        job.lease_id,
        left,
        claim_seconds,
        lowest,
        highest
    )
    .fetch_all(pool)
    .await
}

async fn next_waiting(pool: &PgPool, pass: Pass) -> Result<Option<DateTime<Utc>>, sqlx::Error> {
    match pass {
        Pass::Star(stored) => {
            sqlx::query_file_scalar!(
                "queries/lyrics/lookup_next_star.sql",
                stored,
                STAR_SCAN_WINDOW
            )
            .fetch_optional(pool)
            .await
        }
        Pass::Liked => {
            sqlx::query_file_scalar!(
                "queries/lyrics/lookup_next_priority.sql",
                LIKED_PRIORITY,
                LIKED_PRIORITY
            )
            .fetch_optional(pool)
            .await
        }
        Pass::Rest => {
            sqlx::query_file_scalar!("queries/lyrics/lookup_next_priority.sql", 0, i16::MAX)
                .fetch_optional(pool)
                .await
        }
    }
}
