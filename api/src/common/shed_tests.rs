use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::extract::Request;
use axum::http::{StatusCode, header};
use axum::middleware::from_fn_with_state;
use axum::routing::get;
use serde_json::Value;
use tower::ServiceExt;

use super::*;

async fn slow() -> &'static str {
    tokio::time::sleep(Duration::from_millis(400)).await;
    "ok"
}

fn app(shed: Arc<Shed>) -> Router {
    Router::new()
        .route("/tracks/{id}", get(slow))
        .route("/health", get(slow))
        .layer(from_fn_with_state(shed, super::shed))
}

fn get_request(uri: &str) -> Request {
    Request::builder()
        .uri(uri)
        .body(Body::empty())
        .expect("a request")
}

#[tokio::test]
async fn requests_past_the_limit_get_a_json_503_with_retry_after() {
    let app = app(Shed::new(1, 2, Duration::from_millis(50)));
    let running: Vec<_> = (0..2)
        .map(|_| {
            let app = app.clone();
            tokio::spawn(async move { app.oneshot(get_request("/tracks/1")).await })
        })
        .collect();
    tokio::time::sleep(Duration::from_millis(100)).await;

    let rejected = app
        .clone()
        .oneshot(get_request("/tracks/1"))
        .await
        .expect("answered");
    assert_eq!(rejected.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        rejected.headers().get(header::CONTENT_TYPE),
        Some(&header::HeaderValue::from_static("application/json"))
    );
    let retry_after = rejected
        .headers()
        .get(header::RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .expect("Retry-After is present");
    assert_eq!(retry_after, "3");
    assert!(retry_after.chars().all(|c| c.is_ascii_digit()));
    let body = axum::body::to_bytes(rejected.into_body(), usize::MAX)
        .await
        .expect("body");
    let body: Value = serde_json::from_slice(&body).expect("json");
    assert_eq!(body["statusCode"], 503);
    assert_eq!(body["code"], "api_overloaded");
    assert!(body["message"].as_str().is_some_and(|m| !m.is_empty()));

    for call in running {
        let done = call.await.expect("joined").expect("answered");
        assert_eq!(done.status(), StatusCode::OK);
    }
}

#[tokio::test]
async fn a_waiter_gets_the_slot_when_it_frees_inside_the_wait() {
    let app = app(Shed::new(1, 1, Duration::from_secs(2)));
    let first = {
        let app = app.clone();
        tokio::spawn(async move { app.oneshot(get_request("/tracks/1")).await })
    };
    tokio::time::sleep(Duration::from_millis(50)).await;
    let second = app
        .oneshot(get_request("/tracks/2"))
        .await
        .expect("answered");
    assert_eq!(second.status(), StatusCode::OK);
    first.await.expect("joined").expect("answered");
}

#[tokio::test]
async fn health_and_probe_never_wait_for_a_slot() {
    let app = app(Shed::new(1, 1, Duration::from_millis(20)));
    let held = {
        let app = app.clone();
        tokio::spawn(async move { app.oneshot(get_request("/tracks/1")).await })
    };
    tokio::time::sleep(Duration::from_millis(50)).await;
    let health = app.oneshot(get_request("/health")).await.expect("answered");
    assert_eq!(health.status(), StatusCode::OK);
    held.await.expect("joined").expect("answered");
}

#[test]
fn the_limit_scales_with_the_pool() {
    let shed = Shed::new(18, 4, Duration::from_millis(1));
    assert_eq!(shed.permits.available_permits(), 72);
    assert_eq!(shed.in_flight(), 0);
}

#[test]
fn only_cheap_routes_skip_the_core_limit() {
    assert!(skips_core(&Method::GET, "/health/live"));
    assert!(skips_core(&Method::GET, "/probe"));
    assert!(skips_core(&Method::OPTIONS, "/tracks/1"));
    assert!(!skips_core(&Method::GET, "/tracks/1"));
    assert!(!skips_core(&Method::GET, "/auth/refresh"));
}

#[test]
fn the_room_long_poll_skips_the_core_limit_and_room_writes_do_not() {
    assert!(skips_core(&Method::GET, "/rooms/AB12CD"));
    assert!(!skips_core(&Method::GET, "/rooms/public"));
    assert!(!skips_core(&Method::GET, "/rooms/"));
    assert!(!skips_core(&Method::POST, "/rooms"));
    assert!(!skips_core(&Method::PUT, "/rooms/AB12CD/playback"));
    assert!(!skips_core(&Method::POST, "/rooms/AB12CD/members"));
    assert!(!skips_core(&Method::GET, "/rooms/AB12CD/members"));
}
