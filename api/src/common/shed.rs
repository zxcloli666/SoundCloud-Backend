use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::extract::{MatchedPath, Request, State};
use axum::http::Method;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use tokio::sync::Semaphore;

use crate::error::AppError;

const RETRY_AFTER_SEC: i64 = 3;
const REQUEST_DEADLINE: Duration = Duration::from_secs(60);

pub struct Shed {
    permits: Arc<Semaphore>,
    wait: Duration,
    limit: usize,
}

impl Shed {
    pub fn new(pool_max: u32, factor: u32, wait: Duration) -> Arc<Self> {
        let limit = (pool_max.max(1) as usize) * (factor.max(1) as usize);
        Arc::new(Self {
            permits: Arc::new(Semaphore::new(limit)),
            wait,
            limit,
        })
    }

    pub fn in_flight(&self) -> usize {
        self.limit - self.permits.available_permits()
    }
}

pub fn skips_core(method: &Method, path: &str) -> bool {
    method == Method::OPTIONS
        || path == "/health"
        || path == "/health/live"
        || path == "/probe"
        || path == "/admin/metrics"
        || path == "/tracks/upload"
        || path.starts_with("/.well-known/")
}

pub async fn shed(State(shed): State<Arc<Shed>>, req: Request, next: Next) -> Response {
    if skips_core(req.method(), req.uri().path()) {
        return next.run(req).await;
    }
    let route = crate::common::http_metrics::route_label(req.extensions().get::<MatchedPath>());
    let method = crate::common::http_metrics::method_label(req.method());
    let started = Instant::now();
    let permit = tokio::time::timeout(shed.wait, shed.permits.clone().acquire_owned()).await;
    let Ok(Ok(permit)) = permit else {
        crate::metrics::record_shed(&route);
        let response = AppError::overloaded(RETRY_AFTER_SEC).into_response();
        crate::metrics::record_request(
            method,
            route,
            response.status().as_u16(),
            started.elapsed(),
        );
        return response;
    };
    crate::metrics::set_shed_in_flight(shed.in_flight());
    let response = next.run(req).await;
    drop(permit);
    crate::metrics::set_shed_in_flight(shed.in_flight());
    response
}

pub async fn deadline(req: Request, next: Next) -> Response {
    match tokio::time::timeout(REQUEST_DEADLINE, next.run(req)).await {
        Ok(response) => response,
        Err(_) => AppError::request_timed_out().into_response(),
    }
}

#[cfg(test)]
#[path = "shed_tests.rs"]
mod tests;
