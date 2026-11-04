use std::time::Instant;

use axum::extract::{MatchedPath, Request};
use axum::middleware::Next;
use axum::response::Response;

use crate::common::http_metrics::{method_label, route_label};

pub const INNER_REQUESTS: &str = "chat_inner_requests_total";
pub const INNER_SECONDS: &str = "chat_inner_request_duration_seconds";
pub const PG_QUERIES: &str = "chat_pg_queries_total";
pub const BACKGROUND: &str = "background";

tokio::task_local! {
    static ROUTE: String;
}

pub async fn track(request: Request, next: Next) -> Response {
    let method = method_label(request.method());
    let route = route_label(request.extensions().get::<MatchedPath>());
    let started = Instant::now();
    let response = ROUTE.scope(route.clone(), next.run(request)).await;
    metrics::histogram!(INNER_SECONDS, "method" => method, "route" => route.clone())
        .record(started.elapsed().as_secs_f64());
    metrics::counter!(
        INNER_REQUESTS,
        "method" => method,
        "route" => route,
        "status" => response.status().as_u16().to_string(),
    )
    .increment(1);
    response
}

pub fn pg_query() {
    let route = ROUTE
        .try_with(String::clone)
        .unwrap_or_else(|_| BACKGROUND.to_owned());
    metrics::counter!(PG_QUERIES, "route" => route).increment(1);
}
