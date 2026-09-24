use std::collections::HashMap;
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use deadpool_redis::Pool as RedisPool;
use deadpool_redis::redis::Script;
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use tracing::warn;
use uuid::Uuid;

use crate::error::{AppError, AppResult};

const WINDOW_SECONDS: u64 = 5 * 60;
const MINIMUM_SAMPLES: i64 = 10;
const UNHEALTHY_RATIO: f64 = 0.5;
const REDIS_OPERATION_TIMEOUT: Duration = Duration::from_millis(200);
const INCREMENT_SCRIPT: &str = r#"
local value = redis.call('INCR', KEYS[1])
local ttl = redis.call('PTTL', KEYS[1])
if ttl < 1 then
    redis.call('PEXPIRE', KEYS[1], ARGV[1])
end
return value
"#;

#[derive(Debug, Clone, Default)]
pub struct AppHealth {
    pub successes: i64,
    pub failures: i64,
}

impl AppHealth {
    pub fn unhealthy(&self) -> bool {
        let total = self.successes + self.failures;
        total >= MINIMUM_SAMPLES && self.failures as f64 / total as f64 > UNHEALTHY_RATIO
    }
}

pub struct AuthHealthService {
    redis: RedisPool,
    database: PgPool,
}

impl AuthHealthService {
    pub fn with_database(redis: RedisPool, database: PgPool) -> Arc<Self> {
        Arc::new(Self { redis, database })
    }

    pub async fn run_oauth_request<T, E, F>(&self, oauth_app_id: Uuid, operation: F) -> AppResult<T>
    where
        E: Into<AppError>,
        F: Future<Output = Result<T, E>>,
    {
        if let Some(retry_after) = self.app_penalty_fail_open(oauth_app_id).await {
            return Err(AppError::soundcloud_refresh_rate_limited(retry_after));
        }
        operation.await.map_err(Into::into)
    }

    pub async fn record_app_success(&self, app_id: &str) -> AppResult<()> {
        let key = format!("auth:app:{app_id}:success");
        self.bounded_redis_operation(
            "record success",
            self.increment_expiring(&key, WINDOW_SECONDS),
        )
        .await?;
        Ok(())
    }

    pub async fn record_app_failure(&self, app_id: &str) -> AppResult<()> {
        let key = format!("auth:app:{app_id}:failure");
        self.bounded_redis_operation(
            "record failure",
            self.increment_expiring(&key, WINDOW_SECONDS),
        )
        .await?;
        Ok(())
    }

    pub async fn penalize_app_at_least(
        &self,
        oauth_app_id: Uuid,
        minimum_seconds: u64,
    ) -> AppResult<u64> {
        Ok(self
            .penalize_database(oauth_app_id, minimum_seconds)
            .await?
            .max(1) as u64)
    }

    pub async fn app_healths(&self, app_ids: &[String]) -> AppResult<HashMap<String, AppHealth>> {
        if app_ids.is_empty() {
            return Ok(HashMap::new());
        }
        let mut connection = self.redis.get().await?;
        let mut pipeline = deadpool_redis::redis::pipe();
        for app_id in app_ids {
            pipeline
                .get(format!("auth:app:{app_id}:success"))
                .get(format!("auth:app:{app_id}:failure"));
        }
        let values: Vec<Option<String>> = pipeline.query_async(&mut connection).await?;
        let health = app_ids
            .iter()
            .enumerate()
            .map(|(index, app_id)| {
                let successes = counter_at(&values, index * 2);
                let failures = counter_at(&values, index * 2 + 1);
                (
                    app_id.clone(),
                    AppHealth {
                        successes,
                        failures,
                    },
                )
            })
            .collect();
        Ok(health)
    }

    pub async fn app_penalties_for_apps_fail_open(&self, apps: &[Uuid]) -> HashMap<Uuid, i64> {
        if apps.is_empty() {
            return HashMap::new();
        }
        self.database_penalties_fail_open(apps).await
    }

    pub async fn app_healths_fail_open(&self, app_ids: &[String]) -> HashMap<String, AppHealth> {
        match tokio::time::timeout(REDIS_OPERATION_TIMEOUT, self.app_healths(app_ids)).await {
            Ok(Ok(health)) => health,
            Ok(Err(error)) => {
                warn!(%error, "OAuth app health lookup failed");
                HashMap::new()
            }
            Err(_) => {
                warn!(
                    timeout_ms = REDIS_OPERATION_TIMEOUT.as_millis(),
                    "OAuth app health lookup timed out"
                );
                HashMap::new()
            }
        }
    }

    async fn increment_expiring(&self, key: &str, ttl_seconds: u64) -> AppResult<i64> {
        let mut connection = self.redis.get().await?;
        let ttl_milliseconds = i64::try_from(ttl_seconds.saturating_mul(1_000)).unwrap_or(i64::MAX);
        Ok(Script::new(INCREMENT_SCRIPT)
            .key(key)
            .arg(ttl_milliseconds)
            .invoke_async(&mut connection)
            .await?)
    }

    async fn app_penalty_fail_open(&self, oauth_app_id: Uuid) -> Option<i64> {
        self.app_penalties_for_apps_fail_open(&[oauth_app_id])
            .await
            .get(&oauth_app_id)
            .copied()
    }

    async fn database_penalties_fail_open(&self, apps: &[Uuid]) -> HashMap<Uuid, i64> {
        match sqlx::query_file!("queries/auth/health/app_cooldowns.sql", apps)
            .fetch_all(&self.database)
            .await
        {
            Ok(rows) => rows
                .into_iter()
                .map(|row| (row.oauth_app_id, row.retry_after_seconds))
                .collect(),
            Err(error) => {
                warn!(%error, "OAuth app cooldown database lookup failed");
                HashMap::new()
            }
        }
    }

    async fn penalize_database(&self, oauth_app_id: Uuid, minimum_seconds: u64) -> AppResult<i64> {
        let minimum_seconds = i64::try_from(minimum_seconds).unwrap_or(i64::MAX);
        let retry_after = sqlx::query_file_scalar!(
            "queries/auth/health/penalize_app.sql",
            oauth_app_id,
            minimum_seconds
        )
        .fetch_one(&self.database)
        .await?;
        Ok(retry_after)
    }

    async fn bounded_redis_operation<T, F>(
        &self,
        operation: &'static str,
        future: F,
    ) -> AppResult<T>
    where
        F: Future<Output = AppResult<T>>,
    {
        match tokio::time::timeout(REDIS_OPERATION_TIMEOUT, future).await {
            Ok(result) => result,
            Err(_) => {
                warn!(
                    operation,
                    timeout_ms = REDIS_OPERATION_TIMEOUT.as_millis(),
                    "OAuth health Redis operation timed out"
                );
                Err(AppError::internal("OAuth health store timed out"))
            }
        }
    }
}

pub fn oauth_app_key(client_id: &str) -> String {
    format!(
        "client:{}",
        hex::encode(Sha256::digest(client_id.trim().as_bytes()))
    )
}

fn counter_at(values: &[Option<String>], index: usize) -> i64 {
    values
        .get(index)
        .and_then(Option::as_deref)
        .and_then(|value| value.parse().ok())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use deadpool_redis::{Config, Runtime};
    use futures::future::join_all;
    use tokio::net::TcpListener;

    use super::*;

    #[test]
    fn client_id_has_one_private_identity() {
        let key = oauth_app_key("private-client-id");

        assert_eq!(key, oauth_app_key(" private-client-id "));
        assert!(!key.contains("private-client-id"));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_cooldown_shorter_than_a_second_still_reads_as_one(
        pool: PgPool,
    ) -> anyhow::Result<()> {
        let health = AuthHealthService::with_database(dead_redis()?, pool.clone());
        let oauth_app_id = seed_app(&pool).await?;
        health.penalize_app_at_least(oauth_app_id, 1).await?;

        sqlx::query(
            "UPDATE oauth_app_request_cooldowns SET retry_at = now() + interval '900 milliseconds'",
        )
        .execute(&pool)
        .await?;

        assert_eq!(
            health
                .app_penalties_for_apps_fail_open(&[oauth_app_id])
                .await
                .get(&oauth_app_id),
            Some(&1),
            "Retry-After rounds up, a sub-second cooldown must never be reported as zero"
        );
        Ok(())
    }

    async fn seed_app(pool: &PgPool) -> anyhow::Result<Uuid> {
        let id = Uuid::now_v7();
        sqlx::query(
            "INSERT INTO oauth_apps (id, name, client_id, client_secret, redirect_uri)
             VALUES ($1, 'test', $2, 'secret', 'http://127.0.0.1/cb')",
        )
        .bind(id)
        .bind(id.to_string())
        .execute(pool)
        .await?;
        Ok(id)
    }

    async fn stalled_redis() -> anyhow::Result<(RedisPool, tokio::task::JoinHandle<()>)> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let address = listener.local_addr()?;
        let server = tokio::spawn(async move {
            let (_socket, _) = listener.accept().await.expect("connection should arrive");
            std::future::pending::<()>().await;
        });
        let pool =
            Config::from_url(format!("redis://{address}")).create_pool(Some(Runtime::Tokio1))?;
        Ok((pool, server))
    }

    fn dead_redis() -> anyhow::Result<RedisPool> {
        Ok(Config::from_url("redis://127.0.0.1:1").create_pool(Some(Runtime::Tokio1))?)
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn concurrent_penalties_never_shorten_the_cooldown(pool: PgPool) -> anyhow::Result<()> {
        let health = AuthHealthService::with_database(dead_redis()?, pool.clone());
        let oauth_app_id = seed_app(&pool).await?;

        let updates = [60, 600, 120, 300].map(|minimum| {
            let health = Arc::clone(&health);
            async move { health.penalize_app_at_least(oauth_app_id, minimum).await }
        });
        join_all(updates)
            .await
            .into_iter()
            .collect::<AppResult<Vec<_>>>()?;

        let penalties = health
            .app_penalties_for_apps_fail_open(&[oauth_app_id])
            .await;

        assert!(
            penalties
                .get(&oauth_app_id)
                .is_some_and(|seconds| *seconds >= 600),
            "a shorter penalty must never cut a longer one, saw {penalties:?}"
        );
        Ok(())
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn late_success_does_not_clear_an_active_penalty(pool: PgPool) -> anyhow::Result<()> {
        let health = AuthHealthService::with_database(dead_redis()?, pool.clone());
        let oauth_app_id = seed_app(&pool).await?;

        health.penalize_app_at_least(oauth_app_id, 300).await?;
        let _ = health.record_app_success(&oauth_app_id.to_string()).await;

        let penalties = health
            .app_penalties_for_apps_fail_open(&[oauth_app_id])
            .await;

        assert!(
            penalties
                .get(&oauth_app_id)
                .is_some_and(|seconds| *seconds > 0),
            "a success arriving late must not lift a live cooldown"
        );
        Ok(())
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_cooldown_that_lapsed_stops_holding_the_app_back(pool: PgPool) -> anyhow::Result<()> {
        let health = AuthHealthService::with_database(dead_redis()?, pool.clone());
        let oauth_app_id = seed_app(&pool).await?;
        health.penalize_app_at_least(oauth_app_id, 300).await?;

        sqlx::query(
            "UPDATE oauth_app_request_cooldowns SET retry_at = now() - interval '1 second'",
        )
        .execute(&pool)
        .await?;

        assert!(
            health
                .app_penalties_for_apps_fail_open(&[oauth_app_id])
                .await
                .is_empty(),
            "a cooldown in the past must not read as active"
        );
        Ok(())
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_stalled_redis_is_not_on_the_oauth_path_at_all(pool: PgPool) -> anyhow::Result<()> {
        let (redis, stalled_server) = stalled_redis().await?;
        let health = AuthHealthService::with_database(redis, pool.clone());
        let oauth_app_id = seed_app(&pool).await?;

        let hang_guard = Duration::from_secs(5);
        let result = tokio::time::timeout(
            hang_guard,
            health.run_oauth_request(oauth_app_id, async { Ok::<_, AppError>(42) }),
        )
        .await??;
        let penalty =
            tokio::time::timeout(hang_guard, health.penalize_app_at_least(oauth_app_id, 60))
                .await?;
        stalled_server.abort();

        assert_eq!(
            result, 42,
            "a stalled Redis must not refuse an OAuth request"
        );
        assert!(
            penalty.is_ok_and(|seconds| seconds >= 60),
            "a cooldown lives in PostgreSQL now: routing it back through Redis would make this \
             fail against a server that never answers"
        );
        Ok(())
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_stalled_redis_still_bounds_the_health_counters(pool: PgPool) -> anyhow::Result<()> {
        let (redis, server) = stalled_redis().await?;
        let health = AuthHealthService::with_database(redis, pool);

        let success =
            tokio::time::timeout(Duration::from_secs(1), health.record_app_success("app")).await?;
        server.abort();

        assert!(
            success.is_err(),
            "a health write must give up on its own budget instead of hanging"
        );
        Ok(())
    }
}
