use std::time::Duration;

use axum::Router;
use axum::extract::{MatchedPath, Request};
use axum::http::header::CACHE_CONTROL;
use axum::http::{HeaderValue, StatusCode};
use axum::middleware::{self, Next};
use axum::response::Response;
use tower_http::timeout::TimeoutLayer;
use tower_http::trace::TraceLayer;

use crate::common::http_metrics::{method_label, route_label};
use crate::modules::{chat, health, rooms};
use crate::rt::state::RtState;

pub fn build(state: RtState) -> Router {
    Router::new()
        .merge(health::router())
        .merge(rooms::router())
        .merge(chat::routes::public(state.chat.clone()))
        .layer(TimeoutLayer::with_status_code(
            StatusCode::GATEWAY_TIMEOUT,
            Duration::from_secs(60),
        ))
        .with_state(state)
        .layer(crate::router::body_limit())
        .layer(
            TraceLayer::new_for_http().make_span_with(|request: &Request| {
                tracing::debug_span!(
                    "http_request",
                    method = %request.method(),
                    path = %request.uri().path()
                )
            }),
        )
        .layer(middleware::from_fn(track))
        .layer(middleware::map_response(private))
}

async fn track(request: Request, next: Next) -> Response {
    let method = method_label(request.method());
    let route = route_label(request.extensions().get::<MatchedPath>());
    let started = std::time::Instant::now();
    let response = next.run(request).await;
    crate::metrics::record_request(method, route, response.status().as_u16(), started.elapsed());
    response
}

async fn private(mut response: Response) -> Response {
    response
        .headers_mut()
        .insert(CACHE_CONTROL, HeaderValue::from_static("private, no-store"));
    response
}

#[cfg(test)]
#[path = "router_tests.rs"]
mod tests;
