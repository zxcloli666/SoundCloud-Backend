use sqlx::{PgPool, Postgres, Transaction};
use tracing::info;

use crate::queue::{JobError, JobResult};

mod stage;

use stage::{Shards, Stage};

pub struct DiscoverHandler {
    pool: PgPool,
    always_premium: bool,
    interest_enabled: bool,
    interest_shards: i64,
    artist_plays_shards: i64,
}

impl DiscoverHandler {
    pub fn new(
        pool: PgPool,
        always_premium: bool,
        interest_enabled: bool,
        interest_shards: i64,
        artist_plays_shards: i64,
    ) -> Self {
        Self {
            pool,
            always_premium,
            interest_enabled,
            interest_shards: interest_shards.max(1),
            artist_plays_shards: artist_plays_shards.max(1),
        }
    }

    fn hour(now: chrono::DateTime<chrono::Utc>) -> i64 {
        now.timestamp().div_euclid(3600)
    }

    fn interest_shard(&self, now: chrono::DateTime<chrono::Utc>) -> i64 {
        Self::hour(now).rem_euclid(self.interest_shards)
    }

    pub async fn refresh_aggregates(&self) -> JobResult {
        let started = std::time::Instant::now();
        let shards = Shards::at(self.artist_plays_shards, Self::hour(chrono::Utc::now()));
        for stage in Stage::plan(self.always_premium) {
            self.run(stage, shards).await?;
        }

        info!(
            elapsed_ms = started.elapsed().as_millis() as u64,
            artist_plays_shard = shards.current,
            artist_plays_shards = shards.count,
            "discover aggregates refreshed"
        );
        Ok(())
    }

    pub async fn recompute_interest(&self) -> JobResult {
        if !self.interest_enabled {
            return Ok(());
        }
        let started = std::time::Instant::now();
        let mut transaction = self.locked_transaction().await?;
        let shard = self.interest_shard(chrono::Utc::now());
        let updated = sqlx::query_file!(
            "queries/discover/interest/recompute.sql",
            self.interest_shards,
            shard
        )
        .fetch_one(&mut *transaction)
        .await
        .map_err(JobError::retryable)?;
        let surfaced = sqlx::query_file!("queries/discover/interest/surface_never_crawled.sql")
            .execute(&mut *transaction)
            .await
            .map_err(JobError::retryable)?;
        transaction.commit().await.map_err(JobError::retryable)?;
        info!(
            scored = updated.scored,
            expired = updated.expired,
            shards = self.interest_shards,
            surfaced = surfaced.rows_affected(),
            elapsed_ms = started.elapsed().as_millis() as u64,
            "artist interest recomputed"
        );
        Ok(())
    }

    async fn run(&self, stage: Stage, shards: Shards) -> JobResult {
        let started = std::time::Instant::now();
        let mut transaction = self.locked_transaction().await?;

        stage
            .execute(&mut transaction, shards)
            .await
            .map_err(JobError::retryable)?;
        transaction.commit().await.map_err(JobError::retryable)?;
        tracing::debug!(
            stage = stage.name(),
            elapsed_ms = started.elapsed().as_millis() as u64,
            "discover aggregate stage finished"
        );
        Ok(())
    }

    async fn locked_transaction(&self) -> Result<Transaction<'_, Postgres>, JobError> {
        let mut transaction = self.pool.begin().await.map_err(JobError::retryable)?;
        let acquired = sqlx::query_file_scalar!("queries/discover/try_artist_write_lock.sql")
            .fetch_one(&mut *transaction)
            .await
            .map_err(JobError::retryable)?;

        if !acquired {
            return Err(JobError::retryable(anyhow::anyhow!(
                "another discover artist update is still running"
            )));
        }
        Ok(transaction)
    }
}

#[cfg(test)]
mod tests;
