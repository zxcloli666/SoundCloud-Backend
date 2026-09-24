use axum::Router;
use axum::body::Body;
use axum::extract::Request;
use axum::http::StatusCode;
use axum::routing::post;
use tower::ServiceExt;

use crate::router::{MAX_BODY_BYTES, body_limit};

fn guarded() -> Router {
    Router::new()
        .route(
            "/probe",
            post(|body: axum::body::Bytes| async move { body.len().to_string() }),
        )
        .layer(body_limit())
}

async fn send(bytes: usize) -> StatusCode {
    guarded()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/probe")
                .body(Body::from(vec![b'x'; bytes]))
                .expect("a probe request is well formed"),
        )
        .await
        .expect("the router answers every request")
        .status()
}

#[tokio::test]
async fn a_body_within_the_budget_is_read() {
    assert_eq!(send(MAX_BODY_BYTES).await, StatusCode::OK);
    assert_eq!(send(0).await, StatusCode::OK);
}

#[tokio::test]
async fn a_body_over_the_budget_is_refused_instead_of_buffered() {
    assert_eq!(
        send(MAX_BODY_BYTES + 1).await,
        StatusCode::PAYLOAD_TOO_LARGE,
        "one byte over the budget must be refused; a request nobody bounded is read into \
         memory in full before any handler sees it"
    );
    assert_eq!(
        send(MAX_BODY_BYTES * 8).await,
        StatusCode::PAYLOAD_TOO_LARGE
    );
}

#[test]
fn the_budget_is_the_one_the_router_is_built_with() {
    let router = std::fs::read_to_string(
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/router.rs"),
    )
    .expect("the router is readable");

    assert!(
        router.contains(".layer(body_limit())"),
        "the router no longer applies the body budget; axum's own default would take over \
         silently, and it is not this one"
    );
    let budget = MAX_BODY_BYTES;
    assert!(
        (64 * 1024..=4 * 1024 * 1024).contains(&budget),
        "the budget is {budget} bytes, which is either too small for an ordinary admin \
         payload or large enough to be worth a denial of service"
    );
}
