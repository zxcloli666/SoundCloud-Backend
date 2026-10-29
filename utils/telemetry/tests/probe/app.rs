use std::time::Duration;

use axum::Router;
use axum::extract::{DefaultBodyLimit, Path, Request};
use axum::http::{HeaderName, HeaderValue, StatusCode};
use axum::middleware::{Next, from_fn};
use axum::response::Response;
use axum::routing::get;
use tower_http::compression::CompressionLayer;
use tower_http::cors::{AllowOrigin, CorsLayer};
use tower_http::timeout::TimeoutLayer;
use tower_http::trace::TraceLayer;
use tracing::Instrument;

pub fn build() -> Router {
    Router::new()
        .route("/fast/{id}", get(fast))
        .route("/slow", get(slow))
        .route("/spawned", get(spawned))
        .route("/nested", get(nested))
        .route("/broken", get(broken))
        .layer(TimeoutLayer::with_status_code(
            StatusCode::GATEWAY_TIMEOUT,
            Duration::from_secs(60),
        ))
        .layer(DefaultBodyLimit::max(256 * 1024))
        .layer(CompressionLayer::new())
        .layer(
            TraceLayer::new_for_http().make_span_with(|request: &Request| {
                tracing::debug_span!(
                    "http_request",
                    method = %request.method(),
                    path = %request.uri().path()
                )
            }),
        )
        .layer(from_fn(track_http))
        .layer(from_fn(premium_gate))
        .layer(
            CorsLayer::new()
                .allow_origin(AllowOrigin::mirror_request())
                .allow_headers(tower_http::cors::Any),
        )
        .layer(from_fn(protect))
}

async fn protect(req: Request, next: Next) -> Response {
    let mut response = next.run(req).await;
    response
        .headers_mut()
        .insert("cache-control", HeaderValue::from_static("no-store"));
    response
}

async fn premium_gate(req: Request, next: Next) -> Response {
    tokio::task::yield_now().await;
    next.run(req).await
}

async fn track_http(req: Request, next: Next) -> Response {
    let span = telemetry::root_span(&req);
    let request_id = telemetry::trace_id(&span, req.headers()).unwrap_or_default();
    span.record("request_id", request_id.as_str());
    let mut response = next.run(req).instrument(span.clone()).await;
    telemetry::finish(&span, response.status().as_u16());
    if let Ok(value) = HeaderValue::from_str(&request_id) {
        response
            .headers_mut()
            .insert(HeaderName::from_static("x-request-id"), value);
    }
    response
}

async fn fast(Path(id): Path<u64>) -> String {
    query(1).await;
    id.to_string()
}

async fn query(millis: u64) {
    tokio::time::sleep(Duration::from_millis(millis)).await;
    if log::log_enabled!(target: "sqlx::query", log::Level::Debug) {
        tracing::debug!(target: "sqlx::query", summary = "SELECT 1", rows_returned = 1);
    }
}

async fn slow() -> &'static str {
    query(15).await;
    "slow"
}

async fn spawned() -> String {
    let answer = tokio::spawn(async {
        query(5).await;
        7
    });
    tokio::spawn(async {
        tokio::time::sleep(Duration::from_millis(50)).await;
    });
    answer.await.unwrap_or_default().to_string()
}

async fn nested() -> &'static str {
    lookup().await
}

#[tracing::instrument]
async fn lookup() -> &'static str {
    query(3).await;
    "nested"
}

async fn broken() -> StatusCode {
    query(2).await;
    tracing::warn!("upstream refused");
    telemetry::fail(&tracing::Span::current(), "upstream refused");
    StatusCode::BAD_GATEWAY
}
