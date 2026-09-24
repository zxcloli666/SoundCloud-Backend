use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::RwLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use std::time::Instant;

use anyhow::Context;
use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::get;
use axum::{Json, Router};
use backend_contracts::worker_contract::WorkerLane;
use serde_json::{Value, json};
use sqlx::PgPool;
use tokio_util::sync::CancellationToken;
use tower_http::trace::TraceLayer;

use crate::bus::Bus;
use crate::db::Databases;
use crate::qdrant::QdrantProvisioner;

#[path = "handlers/worker_status.rs"]
pub mod worker_status;

use worker_status::WorkerStatusBoard;

const DATABASE_CHECK_INTERVAL: Duration = Duration::from_secs(5);
const DATABASE_CHECK_TIMEOUT: Duration = Duration::from_secs(2);
const SCHEDULER_STALE_AFTER: Duration = Duration::from_secs(15);

#[derive(Clone)]
pub struct HealthState {
    inner: Arc<HealthInner>,
    metrics_pool: Option<PgPool>,
    workers: WorkerStatusBoard,
}

struct HealthInner {
    live: AtomicBool,
    accepting: AtomicBool,
    main_database: AtomicBool,
    ops_database: AtomicBool,
    nats: AtomicBool,
    qdrant: AtomicBool,
    scheduler_success: RwLock<Option<Instant>>,
}

impl HealthState {
    pub fn with_metrics_pool(mut self, pool: PgPool) -> Self {
        self.metrics_pool = Some(pool);
        self
    }

    pub fn require_worker_lanes(self, lanes: &[WorkerLane]) -> Self {
        self.workers.require_lanes(lanes);
        self
    }

    pub fn new() -> Self {
        Self {
            metrics_pool: None,
            workers: WorkerStatusBoard::new(),
            inner: Arc::new(HealthInner {
                live: AtomicBool::new(true),
                accepting: AtomicBool::new(false),
                main_database: AtomicBool::new(false),
                ops_database: AtomicBool::new(false),
                nats: AtomicBool::new(false),
                qdrant: AtomicBool::new(false),
                scheduler_success: RwLock::new(None),
            }),
        }
    }

    pub fn mark_ready(&self) {
        self.inner.accepting.store(true, Ordering::Release);
    }

    pub fn mark_stopping(&self) {
        self.inner.accepting.store(false, Ordering::Release);
    }

    pub fn is_live(&self) -> bool {
        self.inner.live.load(Ordering::Acquire)
    }

    pub fn is_ready(&self) -> bool {
        self.inner.accepting.load(Ordering::Acquire)
            && self.inner.main_database.load(Ordering::Acquire)
            && self.inner.ops_database.load(Ordering::Acquire)
            && self.inner.nats.load(Ordering::Acquire)
            && self.inner.qdrant.load(Ordering::Acquire)
            && self.scheduler_is_recent()
    }

    fn set_main_database(&self, healthy: bool) -> bool {
        self.inner.main_database.swap(healthy, Ordering::AcqRel) != healthy
    }

    fn set_ops_database(&self, healthy: bool) -> bool {
        self.inner.ops_database.swap(healthy, Ordering::AcqRel) != healthy
    }

    fn set_nats(&self, healthy: bool) -> bool {
        self.inner.nats.swap(healthy, Ordering::AcqRel) != healthy
    }

    fn set_qdrant(&self, healthy: bool) -> bool {
        self.inner.qdrant.swap(healthy, Ordering::AcqRel) != healthy
    }

    pub fn mark_scheduler_success(&self) {
        if let Ok(mut success) = self.inner.scheduler_success.write() {
            *success = Some(Instant::now());
        }
    }

    fn scheduler_is_recent(&self) -> bool {
        self.inner
            .scheduler_success
            .read()
            .ok()
            .and_then(|success| *success)
            .is_some_and(|success| success.elapsed() <= SCHEDULER_STALE_AFTER)
    }
}

impl Default for HealthState {
    fn default() -> Self {
        Self::new()
    }
}

pub async fn serve(
    bind: SocketAddr,
    state: HealthState,
    cancellation: CancellationToken,
) -> anyhow::Result<()> {
    let listener = tokio::net::TcpListener::bind(bind)
        .await
        .with_context(|| format!("failed to bind jobs health server to {bind}"))?;

    axum::serve(listener, router(state))
        .with_graceful_shutdown(cancellation.cancelled_owned())
        .await
        .context("jobs health server failed")
}

pub async fn monitor_dependencies(
    databases: Databases,
    bus: Bus,
    qdrant: QdrantProvisioner,
    state: HealthState,
    cancellation: CancellationToken,
) -> anyhow::Result<()> {
    let workers = state.workers.clone().run(bus.clone(), cancellation.clone());
    let dependencies = watch_dependencies(databases, bus, qdrant, state, cancellation);
    tokio::try_join!(workers, dependencies)?;
    Ok(())
}

async fn watch_dependencies(
    databases: Databases,
    bus: Bus,
    qdrant: QdrantProvisioner,
    state: HealthState,
    cancellation: CancellationToken,
) -> anyhow::Result<()> {
    let mut interval = tokio::time::interval(DATABASE_CHECK_INTERVAL);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    loop {
        tokio::select! {
            biased;
            _ = cancellation.cancelled() => {
                state.mark_stopping();
                return Ok(());
            }
            _ = interval.tick() => {
                let (main, ops, nats, qdrant) = tokio::join!(
                    database_available(&databases.main.fast),
                    database_available(&databases.ops.fast),
                    bus.is_available(),
                    qdrant.is_available(),
                );

                if state.set_main_database(main) {
                    tracing::info!(healthy = main, database = "main", "jobs database health changed");
                }
                if state.set_ops_database(ops) {
                    tracing::info!(healthy = ops, database = "ops", "jobs database health changed");
                }
                if state.set_nats(nats) {
                    tracing::info!(healthy = nats, dependency = "nats", "jobs dependency health changed");
                }
                if state.set_qdrant(qdrant) {
                    tracing::info!(healthy = qdrant, dependency = "qdrant", "jobs dependency health changed");
                }
            }
        }
    }
}

fn router(state: HealthState) -> Router {
    Router::new()
        .route("/live", get(live))
        .route("/ready", get(ready))
        .route("/health", get(health))
        .route("/metrics", get(metrics))
        .with_state(state)
        .layer(TraceLayer::new_for_http())
}

async fn metrics(State(state): State<HealthState>) -> axum::response::Response {
    use axum::response::IntoResponse;

    let Some(pool) = state.metrics_pool.as_ref() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "metrics pool is not wired").into_response();
    };
    match crate::metrics::render(pool).await {
        Some(body) => (
            [(
                axum::http::header::CONTENT_TYPE,
                "text/plain; version=0.0.4; charset=utf-8",
            )],
            crate::metrics::with_node_series(body, state.workers.node_series(Instant::now())),
        )
            .into_response(),
        None => (
            StatusCode::SERVICE_UNAVAILABLE,
            "metrics recorder is not installed",
        )
            .into_response(),
    }
}

async fn live(State(state): State<HealthState>) -> StatusCode {
    status(state.is_live())
}

async fn ready(State(state): State<HealthState>) -> StatusCode {
    status(state.is_ready())
}

async fn health(State(state): State<HealthState>) -> (StatusCode, Json<Value>) {
    let ready = state.is_ready();
    let workers = state.workers.health(Instant::now());
    let body = json!({
        "ready": ready,
        "workers": {
            "healthy": workers.is_healthy(),
            "stale_lanes": workers.stale_lanes,
            "pipeline_done_fill_ratio": workers.done_fill_ratio,
        },
    });
    (status(ready && workers.is_healthy()), Json(body))
}

fn status(healthy: bool) -> StatusCode {
    if healthy {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    }
}

async fn database_available(pool: &PgPool) -> bool {
    matches!(
        tokio::time::timeout(
            DATABASE_CHECK_TIMEOUT,
            sqlx::query_scalar::<_, i32>("SELECT 1").fetch_one(pool),
        )
        .await,
        Ok(Ok(1))
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn readiness_requires_startup_and_both_databases() {
        let state = HealthState::new();

        assert_eq!(live(State(state.clone())).await, StatusCode::OK);
        assert_eq!(
            ready(State(state.clone())).await,
            StatusCode::SERVICE_UNAVAILABLE
        );

        state.set_main_database(true);
        state.set_ops_database(true);
        state.set_nats(true);
        state.set_qdrant(true);
        state.mark_scheduler_success();
        state.mark_ready();

        assert_eq!(ready(State(state.clone())).await, StatusCode::OK);

        state.mark_stopping();

        assert_eq!(ready(State(state)).await, StatusCode::SERVICE_UNAVAILABLE);
    }

    fn ready_state() -> HealthState {
        let state = HealthState::new();
        state.set_main_database(true);
        state.set_ops_database(true);
        state.set_nats(true);
        state.set_qdrant(true);
        state.mark_scheduler_success();
        state.mark_ready();
        state
    }

    #[tokio::test]
    async fn health_follows_readiness_while_no_worker_lane_is_required() {
        let (code, Json(body)) = health(State(ready_state())).await;

        assert_eq!(code, StatusCode::OK);
        assert_eq!(body["workers"]["stale_lanes"], json!([]));
    }

    #[tokio::test]
    async fn health_fails_when_a_required_worker_lane_has_no_fresh_status() -> anyhow::Result<()> {
        let state = ready_state().require_worker_lanes(&[WorkerLane::Audio]);
        let a_minute_ago = Instant::now()
            .checked_sub(worker_status::STATUS_FRESH_FOR)
            .context("the clock is younger than a minute")?;
        state.workers.record_status(
            "gpu-main",
            br#"{"trust":"trusted","lanes":{"audio":{"state":"serving"}}}"#,
            a_minute_ago,
        )?;

        let (code, Json(body)) = health(State(state.clone())).await;

        assert_eq!(code, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(body["workers"]["stale_lanes"], json!(["audio"]));
        assert_eq!(ready(State(state)).await, StatusCode::OK);
        Ok(())
    }

    #[test]
    fn either_database_makes_readiness_fail() {
        let state = HealthState::new();
        state.mark_ready();
        state.set_main_database(true);
        state.set_ops_database(false);
        state.set_nats(true);
        state.set_qdrant(true);
        state.mark_scheduler_success();

        assert!(!state.is_ready());
    }
}

#[cfg(test)]
#[path = "health_metrics_tests.rs"]
mod health_metrics_tests;
