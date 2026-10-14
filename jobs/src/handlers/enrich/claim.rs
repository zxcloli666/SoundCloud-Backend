#[cfg(test)]
#[path = "claim_tests.rs"]
mod tests;

use chrono::{DateTime, Utc};
use sqlx::PgPool;

use super::{ClaimedTrack, LEASE_TIMEOUT};
use crate::handlers::star_priority::{LIKED_PRIORITY, Pass, Round, STAR_SCAN_WINDOW, Turn};

pub(super) struct Claim {
    pub(super) batch: i64,
    pub(super) max_attempts: i16,
}

impl Claim {
    pub(super) async fn take(
        &self,
        pool: &PgPool,
        turn: Turn,
    ) -> Result<Vec<ClaimedTrack>, sqlx::Error> {
        let mut round = Round::new("enrich");
        let mut claimed = Vec::new();
        for tier in turn.tiers() {
            let left = self.batch - i64::try_from(claimed.len()).unwrap_or(i64::MAX);
            let found = if left > 0 {
                self.claim_pass(pool, tier.pass(), left).await?
            } else {
                Vec::new()
            };
            let drained = i64::try_from(found.len()).unwrap_or(i64::MAX) < left;
            let next = if drained {
                None
            } else {
                self.next_waiting(pool, tier.pass()).await?
            };
            round.record(tier.label(), found.len(), next);
            claimed.extend(found);
        }
        round.report();
        Ok(claimed)
    }

    async fn claim_pass(
        &self,
        pool: &PgPool,
        pass: Pass,
        left: i64,
    ) -> Result<Vec<ClaimedTrack>, sqlx::Error> {
        match pass {
            Pass::Star(stored) => {
                sqlx::query_file_as!(
                    ClaimedTrack,
                    "queries/enrich/source/claim_star.sql",
                    lease_seconds(),
                    self.max_attempts,
                    left,
                    stored,
                    STAR_SCAN_WINDOW
                )
                .fetch_all(pool)
                .await
            }
            Pass::Liked => {
                self.claim_by_priority(pool, left, LIKED_PRIORITY, LIKED_PRIORITY)
                    .await
            }
            Pass::Rest => self.claim_by_priority(pool, left, 0, i16::MAX).await,
        }
    }

    async fn claim_by_priority(
        &self,
        pool: &PgPool,
        left: i64,
        lowest: i16,
        highest: i16,
    ) -> Result<Vec<ClaimedTrack>, sqlx::Error> {
        sqlx::query_file_as!(
            ClaimedTrack,
            "queries/enrich/source/claim_batch.sql",
            lease_seconds(),
            self.max_attempts,
            left,
            lowest,
            highest
        )
        .fetch_all(pool)
        .await
    }

    async fn next_waiting(
        &self,
        pool: &PgPool,
        pass: Pass,
    ) -> Result<Option<DateTime<Utc>>, sqlx::Error> {
        match pass {
            Pass::Star(stored) => {
                sqlx::query_file_scalar!(
                    "queries/enrich/source/next_star.sql",
                    lease_seconds(),
                    self.max_attempts,
                    stored,
                    STAR_SCAN_WINDOW
                )
                .fetch_optional(pool)
                .await
            }
            Pass::Liked => {
                self.next_by_priority(pool, LIKED_PRIORITY, LIKED_PRIORITY)
                    .await
            }
            Pass::Rest => self.next_by_priority(pool, 0, i16::MAX).await,
        }
    }

    async fn next_by_priority(
        &self,
        pool: &PgPool,
        lowest: i16,
        highest: i16,
    ) -> Result<Option<DateTime<Utc>>, sqlx::Error> {
        sqlx::query_file_scalar!(
            "queries/enrich/source/next_priority.sql",
            lease_seconds(),
            self.max_attempts,
            lowest,
            highest
        )
        .fetch_optional(pool)
        .await
    }
}

fn lease_seconds() -> f64 {
    LEASE_TIMEOUT.as_secs_f64()
}
