use std::sync::OnceLock;
use std::time::Duration;

use metrics_exporter_prometheus::{Matcher, PrometheusBuilder, PrometheusHandle};
use sqlx::PgPool;

const REQUEST_DURATION: &str = "api_http_request_duration_seconds";
const REQUESTS_TOTAL: &str = "api_http_requests_total";
const DEPENDENCY_DURATION: &str = "api_dependency_call_duration_seconds";
const DEPENDENCY_TOTAL: &str = "api_dependency_calls_total";
const POOL_CONNECTIONS: &str = "api_pg_pool_connections";
const SC_TIER_TOTAL: &str = "api_sc_tier_calls_total";
const SC_TIER_DURATION: &str = "api_sc_tier_call_duration_seconds";
const SC_RELAY_BREAKER_OPEN: &str = "api_sc_relay_breaker_open";
const SC_FAILURES: &str = "api_sc_failures_total";
const SC_RETRY_AFTER: &str = "api_sc_retry_after_seconds";
const TASTE_VECTOR_READ_ERRORS: &str = "api_taste_vector_read_errors_total";
const PG_BACKENDS: &str = "api_pg_backends";
const PG_TRANSACTIONS: &str = "api_pg_transactions_total";
const PG_DEADLOCKS: &str = "api_pg_deadlocks_total";
const PG_BLOCKS: &str = "api_pg_blocks_total";
const PG_SESSIONS_WAITING: &str = "api_pg_sessions";
const POOL_WAIT: &str = "api_pg_pool_wait_seconds";
const POOL_WAIT_LAST: &str = "api_pg_pool_wait_last_seconds";

const POOL_PROBE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

const POOL_WAIT_BUCKETS: &[f64] = &[
    0.0001, 0.0005, 0.001, 0.005, 0.01, 0.05, 0.1, 0.5, 1.0, 2.5, 5.0,
];

const LATENCY_BUCKETS: &[f64] = &[
    0.001, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0,
];

const RETRY_AFTER_BUCKETS: &[f64] = &[1.0, 5.0, 15.0, 30.0, 60.0, 300.0, 900.0, 3600.0];

static HANDLE: OnceLock<PrometheusHandle> = OnceLock::new();

pub fn init() {
    if HANDLE.get().is_some() {
        return;
    }
    let builder = PrometheusBuilder::new()
        .set_buckets_for_metric(Matcher::Full(REQUEST_DURATION.to_owned()), LATENCY_BUCKETS)
        .and_then(|builder| {
            builder.set_buckets_for_metric(
                Matcher::Full(DEPENDENCY_DURATION.to_owned()),
                LATENCY_BUCKETS,
            )
        })
        .and_then(|builder| {
            builder
                .set_buckets_for_metric(Matcher::Full(SC_TIER_DURATION.to_owned()), LATENCY_BUCKETS)
        })
        .and_then(|builder| {
            builder.set_buckets_for_metric(Matcher::Full(POOL_WAIT.to_owned()), POOL_WAIT_BUCKETS)
        })
        .and_then(|builder| {
            builder.set_buckets_for_metric(
                Matcher::Full(SC_RETRY_AFTER.to_owned()),
                RETRY_AFTER_BUCKETS,
            )
        });
    let builder = match builder {
        Ok(builder) => builder,
        Err(error) => {
            tracing::warn!(%error, "metrics buckets rejected");
            return;
        }
    };
    match builder.install_recorder() {
        Ok(handle) => {
            let _ = HANDLE.set(handle);
        }
        Err(error) => tracing::warn!(%error, "metrics recorder unavailable"),
    }
}

pub fn record_request(method: &'static str, route: String, status: u16, elapsed: Duration) {
    if HANDLE.get().is_none() {
        return;
    }
    let status = status.to_string();
    metrics::histogram!(
        REQUEST_DURATION,
        "method" => method,
        "route" => route.clone(),
    )
    .record(elapsed.as_secs_f64());
    metrics::counter!(
        REQUESTS_TOTAL,
        "method" => method,
        "route" => route,
        "status" => status,
    )
    .increment(1);
}

pub fn record_dependency(
    dependency: &'static str,
    operation: &'static str,
    outcome: Outcome,
    elapsed: Duration,
) {
    if HANDLE.get().is_none() {
        return;
    }
    metrics::histogram!(
        DEPENDENCY_DURATION,
        "dependency" => dependency,
        "operation" => operation,
    )
    .record(elapsed.as_secs_f64());
    metrics::counter!(
        DEPENDENCY_TOTAL,
        "dependency" => dependency,
        "operation" => operation,
        "outcome" => outcome.as_str(),
    )
    .increment(1);
}

pub fn record_sc_tier(
    tier: &'static str,
    operation: &'static str,
    outcome: Outcome,
    elapsed: Duration,
) {
    if HANDLE.get().is_none() {
        return;
    }
    metrics::histogram!(SC_TIER_DURATION, "tier" => tier, "operation" => operation)
        .record(elapsed.as_secs_f64());
    metrics::counter!(
        SC_TIER_TOTAL,
        "tier" => tier,
        "operation" => operation,
        "outcome" => outcome.as_str(),
    )
    .increment(1);
}

pub fn record_sc_failure(error: &crate::error::AppError) {
    if HANDLE.get().is_none() {
        return;
    }
    let class = crate::sc::classify(error).as_str();
    metrics::counter!(SC_FAILURES, "class" => class).increment(1);
    if let Some(seconds) = crate::sc::retry_after_seconds(error) {
        metrics::histogram!(SC_RETRY_AFTER).record(seconds as f64);
    }
}

pub fn record_taste_vector_read_error() {
    if HANDLE.get().is_none() {
        return;
    }
    metrics::counter!(TASTE_VECTOR_READ_ERRORS).increment(1);
}

pub fn set_relay_breaker_open(open: bool) {
    if HANDLE.get().is_none() {
        return;
    }
    metrics::gauge!(SC_RELAY_BREAKER_OPEN).set(f64::from(u8::from(open)));
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Ok,
    Miss,
    Error,
    Timeout,
}

impl Outcome {
    fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Miss => "miss",
            Self::Error => "error",
            Self::Timeout => "timeout",
        }
    }
}

#[cfg(test)]
pub fn render_for_tests() -> Option<String> {
    HANDLE.get().map(PrometheusHandle::render)
}

pub async fn render(pool: &PgPool) -> Option<String> {
    let handle = HANDLE.get()?;
    let size = f64::from(pool.size());
    let idle = pool.num_idle() as f64;
    metrics::gauge!(POOL_CONNECTIONS, "state" => "open").set(size);
    metrics::gauge!(POOL_CONNECTIONS, "state" => "idle").set(idle);
    metrics::gauge!(POOL_CONNECTIONS, "state" => "busy").set((size - idle).max(0.0));
    sample_pool_wait(pool).await;
    sample_database(pool).await;
    Some(handle.render())
}

pub async fn sample_pool_wait(pool: &PgPool) {
    let started = std::time::Instant::now();
    let outcome = match tokio::time::timeout(POOL_PROBE_TIMEOUT, pool.acquire()).await {
        Ok(Ok(connection)) => {
            drop(connection);
            Outcome::Ok
        }
        Ok(Err(_)) => Outcome::Error,
        Err(_) => Outcome::Timeout,
    };
    let waited = started.elapsed().as_secs_f64();
    metrics::histogram!(POOL_WAIT, "outcome" => outcome.as_str()).record(waited);
    metrics::gauge!(POOL_WAIT_LAST).set(waited);
}

async fn sample_database(pool: &PgPool) {
    let database = sqlx::query_as::<_, (i32, i64, i64, i64, i64, i64)>(
        "SELECT numbackends, xact_commit, xact_rollback, deadlocks, blks_hit, blks_read
         FROM pg_stat_database
         WHERE datname = current_database()",
    )
    .fetch_optional(pool)
    .await;
    match database {
        Ok(Some((backends, committed, rolled_back, deadlocks, hit, read))) => {
            metrics::gauge!(PG_BACKENDS).set(f64::from(backends));
            metrics::gauge!(PG_TRANSACTIONS, "outcome" => "commit").set(committed as f64);
            metrics::gauge!(PG_TRANSACTIONS, "outcome" => "rollback").set(rolled_back as f64);
            metrics::gauge!(PG_DEADLOCKS).set(deadlocks as f64);
            metrics::gauge!(PG_BLOCKS, "source" => "cache").set(hit as f64);
            metrics::gauge!(PG_BLOCKS, "source" => "disk").set(read as f64);
        }
        Ok(None) => {}
        Err(error) => {
            tracing::debug!(%error, "database statistics are unavailable");
            return;
        }
    }

    let waits = sqlx::query_as::<_, (String, i64)>(
        "SELECT coalesce(wait_event_type, 'running') AS kind, count(*)
         FROM pg_stat_activity
         WHERE datname = current_database()
         GROUP BY 1",
    )
    .fetch_all(pool)
    .await;
    if let Ok(rows) = waits {
        for (kind, sessions) in rows {
            metrics::gauge!(PG_SESSIONS_WAITING, "wait" => kind).set(sessions as f64);
        }
    }
}

#[cfg(test)]
#[path = "metrics_tests.rs"]
mod tests;
