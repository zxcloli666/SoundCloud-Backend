use std::time::Duration;

use mini_moka::sync::Cache;
use sqlx::PgPool;

const KEPT: Duration = Duration::from_secs(12 * 3600);
const CAPACITY: u64 = 500_000;

pub(super) struct ReadStamps {
    stamped: Cache<String, ()>,
}

impl ReadStamps {
    pub(super) fn new() -> Self {
        Self {
            stamped: Cache::builder()
                .max_capacity(CAPACITY)
                .time_to_live(KEPT)
                .build(),
        }
    }

    pub(super) async fn touch(&self, pg: &PgPool, sc_track_id: &str) {
        let key = sc_track_id.to_owned();
        if self.stamped.contains_key(&key) {
            return;
        }
        let stamped = sqlx::query_file!("queries/tracks/service/touch_last_read.sql", sc_track_id)
            .execute(pg)
            .await;
        if stamped.is_ok_and(|done| done.rows_affected() == 1) {
            self.stamped.insert(key, ());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn seed(pool: &PgPool) -> anyhow::Result<()> {
        sqlx::query(
            "INSERT INTO tracks (sc_track_id, urn, title, title_normalized, duration_ms, sharing)
             VALUES ('7001', 'soundcloud:tracks:7001', 'Stamp', 'stamp', 1000, 'public')",
        )
        .execute(pool)
        .await?;
        Ok(())
    }

    async fn last_read(pool: &PgPool) -> anyhow::Result<Option<chrono::DateTime<chrono::Utc>>> {
        Ok(
            sqlx::query_scalar("SELECT last_read_at FROM tracks WHERE sc_track_id = '7001'")
                .fetch_one(pool)
                .await?,
        )
    }

    async fn plain_sql(pool: &PgPool) -> anyhow::Result<u64> {
        Ok(
            sqlx::query_file!("queries/tracks/service/touch_last_read.sql", "7001")
                .execute(pool)
                .await?
                .rows_affected(),
        )
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_stamp_written_now_matches_what_the_sql_guard_would_do(
        pool: PgPool,
    ) -> anyhow::Result<()> {
        seed(&pool).await?;
        let stamps = ReadStamps::new();

        stamps.touch(&pool, "7001").await;
        let first = last_read(&pool).await?.expect("stamped");

        stamps.touch(&pool, "7001").await;
        assert_eq!(last_read(&pool).await?, Some(first));
        assert_eq!(plain_sql(&pool).await?, 0, "the sql guard also skips it");
        Ok(())
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_noop_update_never_enters_the_cache_so_a_due_stamp_is_not_delayed(
        pool: PgPool,
    ) -> anyhow::Result<()> {
        seed(&pool).await?;
        sqlx::query("UPDATE tracks SET last_read_at = now() - interval '2 hours'")
            .execute(&pool)
            .await?;
        let stamps = ReadStamps::new();

        stamps.touch(&pool, "7001").await;
        assert!(
            stamps.stamped.get(&"7001".to_owned()).is_none(),
            "the guard skipped the row, nothing may be remembered"
        );

        sqlx::query("UPDATE tracks SET last_read_at = now() - interval '25 hours'")
            .execute(&pool)
            .await?;
        let due = last_read(&pool).await?.expect("old stamp");
        stamps.touch(&pool, "7001").await;
        assert!(last_read(&pool).await?.expect("restamped") > due);
        Ok(())
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_failed_update_is_not_remembered(pool: PgPool) -> anyhow::Result<()> {
        seed(&pool).await?;
        let stamps = ReadStamps::new();
        sqlx::query("ALTER TABLE tracks RENAME COLUMN last_read_at TO last_read_at_x")
            .execute(&pool)
            .await?;
        stamps.touch(&pool, "7001").await;
        assert!(stamps.stamped.get(&"7001".to_owned()).is_none());
        Ok(())
    }
}
