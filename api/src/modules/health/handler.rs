use std::time::{Duration, Instant};

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use serde::Serialize;
use sqlx::PgPool;
use tokio::sync::Mutex;
use tracing::warn;

use crate::state::AppState;

const DATABASE_TIMEOUT: Duration = Duration::from_secs(2);
const VERDICT_TTL: Duration = Duration::from_secs(5);
const STALL_WINDOW: Duration = Duration::from_secs(60);

type Verdict = Result<(), &'static str>;

static LAST_VERDICT: Mutex<Option<(Instant, Verdict)>> = Mutex::const_new(None);

#[derive(Serialize)]
pub struct HealthResponse {
    pub status: &'static str,
    pub timestamp: String,
}

pub async fn check(State(state): State<AppState>) -> (StatusCode, Json<HealthResponse>) {
    answer(cached_verdict(&state.pg).await)
}

pub async fn live(State(state): State<AppState>) -> (StatusCode, Json<HealthResponse>) {
    answer(stalled(&state.pg, crate::db::core_pool_quiet_for()).await)
}

fn answer(verdict: Verdict) -> (StatusCode, Json<HealthResponse>) {
    let (code, status) = match verdict {
        Ok(()) => (StatusCode::OK, "ok"),
        Err(reason) => (StatusCode::SERVICE_UNAVAILABLE, reason),
    };
    (
        code,
        Json(HealthResponse {
            status,
            timestamp: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        }),
    )
}

async fn stalled(pool: &PgPool, quiet_for: Duration) -> Verdict {
    if quiet_for < STALL_WINDOW {
        return Ok(());
    }
    if let Ok(Ok(_)) = tokio::time::timeout(DATABASE_TIMEOUT, pool.acquire()).await {
        return Ok(());
    }
    warn!(
        quiet_secs = quiet_for.as_secs(),
        open = pool.size(),
        idle = pool.num_idle(),
        "database pool stalled"
    );
    Err("database_pool_stalled")
}

async fn cached_verdict(pool: &PgPool) -> Verdict {
    let mut last = LAST_VERDICT.lock().await;
    if let Some((checked_at, verdict)) = *last
        && checked_at.elapsed() < VERDICT_TTL
    {
        return verdict;
    }
    let verdict = database(pool).await;
    *last = Some((Instant::now(), verdict));
    verdict
}

async fn database(pool: &PgPool) -> Verdict {
    let verdict = match tokio::time::timeout(DATABASE_TIMEOUT, pool.acquire()).await {
        Ok(Ok(_)) => Ok(()),
        Ok(Err(sqlx::Error::PoolTimedOut)) | Err(_) => Err("database_pool_exhausted"),
        Ok(Err(_)) => Err("database_unavailable"),
    };
    if let Err(reason) = verdict {
        warn!(
            reason,
            open = pool.size(),
            idle = pool.num_idle(),
            "health check failed"
        );
    }
    verdict
}

#[cfg(test)]
mod tests {
    use sqlx::postgres::{PgConnectOptions, PgPoolOptions};

    use super::*;

    #[sqlx::test(migrations = false)]
    async fn a_pool_with_no_free_connection_is_unhealthy(
        _: PgPoolOptions,
        opts: PgConnectOptions,
    ) -> anyhow::Result<()> {
        let pool = PgPoolOptions::new()
            .max_connections(1)
            .acquire_timeout(Duration::from_secs(30))
            .connect_with(opts)
            .await?;
        assert_eq!(database(&pool).await, Ok(()));

        let held = pool.acquire().await?;
        assert_eq!(database(&pool).await, Err("database_pool_exhausted"));

        drop(held);
        assert_eq!(database(&pool).await, Ok(()));
        Ok(())
    }

    #[sqlx::test(migrations = false)]
    async fn a_busy_pool_stays_alive_while_connections_keep_coming_back(
        _: PgPoolOptions,
        opts: PgConnectOptions,
    ) -> anyhow::Result<()> {
        let pool = PgPoolOptions::new()
            .max_connections(1)
            .acquire_timeout(Duration::from_secs(30))
            .connect_with(opts)
            .await?;
        let held = pool.acquire().await?;

        assert_eq!(stalled(&pool, Duration::from_secs(5)).await, Ok(()));
        assert_eq!(
            stalled(&pool, STALL_WINDOW).await,
            Err("database_pool_stalled")
        );

        drop(held);
        assert_eq!(stalled(&pool, STALL_WINDOW).await, Ok(()));
        Ok(())
    }
}
