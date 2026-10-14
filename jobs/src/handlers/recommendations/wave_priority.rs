use chrono::Utc;
use sqlx::PgPool;

use crate::queue::{JobError, JobResult};

pub(super) fn shard_for(shards: i64, now: chrono::DateTime<Utc>) -> i64 {
    let shards = shards.max(1);
    now.timestamp().div_euclid(3600).rem_euclid(shards)
}

pub(super) async fn bump(pool: &PgPool, shards: i64) -> JobResult<u64> {
    let shards = shards.max(1);
    let shard = shard_for(shards, Utc::now());
    let result = sqlx::query_file!(
        "queries/recommendations/wave_priority/bump.sql",
        shards,
        shard
    )
    .execute(pool)
    .await
    .map_err(JobError::retryable)?;
    Ok(result.rows_affected())
}

#[cfg(test)]
mod tests {
    use anyhow::Context;

    use super::*;

    const TEST_SHARDS: i64 = 16;

    #[test]
    fn every_shard_is_visited_once_per_cycle() -> anyhow::Result<()> {
        let start = chrono::DateTime::from_timestamp(0, 0).context("epoch")?;
        let visited: Vec<i64> = (0..TEST_SHARDS)
            .map(|hour| shard_for(TEST_SHARDS, start + chrono::Duration::hours(hour)))
            .collect();
        let mut sorted = visited.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len() as i64, TEST_SHARDS);
        assert_eq!(
            shard_for(TEST_SHARDS, start),
            shard_for(TEST_SHARDS, start + chrono::Duration::hours(TEST_SHARDS))
        );
        Ok(())
    }

    #[test]
    fn a_single_shard_covers_everyone_every_run() -> anyhow::Result<()> {
        let start = chrono::DateTime::from_timestamp(1_700_000_000, 0).context("stamp")?;
        for hour in 0..5 {
            assert_eq!(shard_for(1, start + chrono::Duration::hours(hour)), 0);
        }
        Ok(())
    }

    #[test]
    fn a_nonsensical_shard_count_never_divides_by_zero() -> anyhow::Result<()> {
        let start = chrono::DateTime::from_timestamp(1_700_000_000, 0).context("stamp")?;
        assert_eq!(shard_for(0, start), 0);
        assert_eq!(shard_for(-4, start), 0);
        Ok(())
    }
}
