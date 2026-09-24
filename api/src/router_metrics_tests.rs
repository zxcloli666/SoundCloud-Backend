use std::sync::{Arc, Mutex};

use axum::Router;
use axum::body::Body;
use axum::extract::{MatchedPath, Request};
use axum::middleware::Next;
use axum::routing::get;
use tower::ServiceExt;

use crate::common::http_metrics::route_label;

fn probe() -> (Router, Arc<Mutex<Vec<String>>>) {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let captured = seen.clone();
    let app = Router::new()
        .route("/tracks/{urn}", get(|| async { "ok" }))
        .route("/discover/tags", get(|| async { "ok" }))
        .layer(axum::middleware::from_fn(
            move |req: Request, next: Next| {
                let captured = captured.clone();
                async move {
                    let label = route_label(req.extensions().get::<MatchedPath>());
                    captured.lock().expect("probe is not poisoned").push(label);
                    next.run(req).await
                }
            },
        ));
    (app, seen)
}

async fn visit(app: &Router, uri: &str) {
    app.clone()
        .oneshot(
            Request::builder()
                .uri(uri)
                .body(Body::empty())
                .expect("a probe request is well formed"),
        )
        .await
        .expect("the router answers every request");
}

async fn answer_with_request_id(app: &Router, incoming: Option<&str>) -> axum::http::HeaderMap {
    let mut request = Request::builder().uri("/discover/tags");
    if let Some(incoming) = incoming {
        request = request.header(crate::common::request_id::HEADER, incoming);
    }
    app.clone()
        .oneshot(
            request
                .body(Body::empty())
                .expect("a probe request is well formed"),
        )
        .await
        .expect("the router answers every request")
        .headers()
        .clone()
}

fn echoing_router() -> Router {
    Router::new()
        .route("/discover/tags", get(|| async { "ok" }))
        .layer(axum::middleware::from_fn(
            |req: Request, next: Next| async move {
                let id = crate::common::request_id::accept_or_mint(
                    req.headers()
                        .get(crate::common::request_id::HEADER)
                        .and_then(|value| value.to_str().ok()),
                );
                let mut response = next.run(req).await;
                if let Ok(value) = axum::http::HeaderValue::from_str(&id) {
                    response.headers_mut().insert(
                        axum::http::HeaderName::from_static(crate::common::request_id::HEADER),
                        value,
                    );
                }
                response
            },
        ))
}

#[tokio::test]
async fn every_answer_carries_an_id_a_report_can_be_traced_by() {
    let app = echoing_router();

    let minted = answer_with_request_id(&app, None).await;
    let carried = minted
        .get(crate::common::request_id::HEADER)
        .and_then(|value| value.to_str().ok())
        .expect("every answer names its request");
    assert_eq!(carried.len(), 32);

    let echoed = answer_with_request_id(&app, Some("desktop-7")).await;
    assert_eq!(
        echoed
            .get(crate::common::request_id::HEADER)
            .and_then(|value| value.to_str().ok()),
        Some("desktop-7"),
        "an id the client brought must come back unchanged so both sides name the same request"
    );
}

#[tokio::test]
async fn an_id_a_client_forged_never_reaches_the_answer() {
    let app = echoing_router();

    let headers = answer_with_request_id(&app, Some("forged id with spaces")).await;

    let carried = headers
        .get(crate::common::request_id::HEADER)
        .and_then(|value| value.to_str().ok())
        .expect("every answer names its request");
    assert_ne!(carried, "forged id with spaces");
    assert_eq!(carried.len(), 32, "it was replaced, not patched up");
}

#[tokio::test]
async fn the_route_label_is_the_matched_pattern_and_never_the_requested_path() {
    let (app, seen) = probe();

    visit(&app, "/tracks/soundcloud:tracks:42").await;
    visit(&app, "/discover/tags").await;

    let seen = seen.lock().expect("probe is not poisoned").clone();
    assert_eq!(
        seen,
        vec!["/tracks/{urn}".to_owned(), "/discover/tags".to_owned()],
        "the label must come from the route table, not from what the client asked for"
    );
}

#[tokio::test]
async fn a_path_nobody_routes_collapses_into_one_label() {
    let (app, seen) = probe();

    for nonce in 0..32 {
        visit(&app, &format!("/there-is-no-such-route-{nonce}")).await;
    }

    let seen = seen.lock().expect("probe is not poisoned").clone();
    assert_eq!(seen.len(), 32);
    assert!(
        seen.iter().all(|label| label == "unmatched"),
        "an unrouted path must not mint a time series, saw {seen:?}"
    );
}
