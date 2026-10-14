use uuid::Uuid;

use super::AccountWalkHandler;
use crate::handlers::star_priority::Round;

impl AccountWalkHandler {
    pub(super) async fn claim(&self) -> Result<Vec<Uuid>, sqlx::Error> {
        let batch = self.config.batch;
        let (star, regular) = if self.turns.next().star_first() {
            let star = self.claim_star(batch).await?;
            let regular = self.claim_regular(batch - count(&star)).await?;
            (star, regular)
        } else {
            let regular = self.claim_regular(batch).await?;
            let star = self.claim_star(batch - count(&regular)).await?;
            (star, regular)
        };
        let next = sqlx::query_file!(
            "queries/account_walk/next_waiting.sql",
            self.config.walk_interval_days,
            self.config.lease_seconds
        )
        .fetch_one(&self.pool)
        .await?;
        let mut round = Round::new("account_walk");
        round.record("star", star.len(), next.star);
        round.record("regular", regular.len(), next.regular);
        round.report();
        Ok(star.into_iter().chain(regular).collect())
    }

    async fn claim_star(&self, limit: i64) -> Result<Vec<Uuid>, sqlx::Error> {
        if limit <= 0 {
            return Ok(Vec::new());
        }
        sqlx::query_file_scalar!(
            "queries/account_walk/claim_star.sql",
            self.config.walk_interval_days,
            self.config.lease_seconds,
            limit
        )
        .fetch_all(&self.pool)
        .await
    }

    async fn claim_regular(&self, limit: i64) -> Result<Vec<Uuid>, sqlx::Error> {
        if limit <= 0 {
            return Ok(Vec::new());
        }
        sqlx::query_file_scalar!(
            "queries/account_walk/claim.sql",
            self.config.walk_interval_days,
            self.config.lease_seconds,
            limit
        )
        .fetch_all(&self.pool)
        .await
    }
}

fn count(artists: &[Uuid]) -> i64 {
    i64::try_from(artists.len()).unwrap_or(i64::MAX)
}
