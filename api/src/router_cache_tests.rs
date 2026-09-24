use axum::body::Body;
use axum::extract::Request;
use axum::http::header::{CACHE_CONTROL, PRAGMA, REFERRER_POLICY, VARY};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use tower::ServiceExt;

use super::{CAPABILITY_HEADERS, protect_sensitive_responses};

fn probe() -> Router {
    Router::new()
        .route("/me", get(|| async { Json(serde_json::json!({"me": 1})) }))
        .route("/discover/tags", get(|| async { "ok" }))
        .route("/auth/session", get(|| async { "ok" }))
        .route(
            "/tracks/{urn}/stream",
            get(|| async { StatusCode::FOUND.into_response() }),
        )
        .route(
            "/me/denied",
            get(|| async { StatusCode::UNAUTHORIZED.into_response() }),
        )
        .route(
            "/me/boom",
            get(|| async { StatusCode::INTERNAL_SERVER_ERROR.into_response() }),
        )
        .route("/auth/callback", get(|| async { handing_out_a_session() }))
        .layer(axum::middleware::from_fn(protect_sensitive_responses))
}

fn handing_out_a_session() -> Response {
    let mut response = "ok".into_response();
    response
        .headers_mut()
        .insert("x-session-id", HeaderValue::from_static("a-fresh-session"));
    response
}

async fn answer(uri: &str, capability: Option<(&'static str, &'static str)>) -> Response {
    let mut request = Request::builder().uri(uri);
    if let Some((name, value)) = capability {
        request = request.header(name, value);
    }
    probe()
        .oneshot(
            request
                .body(Body::empty())
                .expect("a probe request is well formed"),
        )
        .await
        .expect("the router answers every request")
}

fn kept_out_of_shared_caches(headers: &HeaderMap) -> bool {
    headers
        .get(CACHE_CONTROL)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.contains("no-store") && value.contains("private"))
}

#[tokio::test]
async fn an_answer_to_a_request_that_showed_a_capability_is_never_stored_by_a_shared_cache() {
    for header in CAPABILITY_HEADERS {
        let response = answer("/me", Some((header, "whatever-it-holds"))).await;
        let headers = response.headers();
        assert!(
            kept_out_of_shared_caches(headers),
            "a request that showed {header} gets a personal answer; `x-session-id` is not \
             `Authorization`, so no cache in front of us treats it as private on its own and \
             one listener's /me can be replayed to the next"
        );
        assert_eq!(
            headers.get(PRAGMA),
            Some(&HeaderValue::from_static("no-cache"))
        );
        assert_eq!(
            headers.get(REFERRER_POLICY),
            Some(&HeaderValue::from_static("no-referrer"))
        );
        assert_eq!(
            headers.get(VARY).and_then(|value| value.to_str().ok()),
            Some("x-session-id, x-admin-token"),
            "a cache that stores the answer anyway must at least not serve it to another holder"
        );
    }
}

#[tokio::test]
async fn a_refusal_and_a_failure_are_protected_exactly_like_the_answer_they_replace() {
    for (uri, expected) in [
        ("/me/denied", StatusCode::UNAUTHORIZED),
        ("/me/boom", StatusCode::INTERNAL_SERVER_ERROR),
        ("/me/there-is-no-such-route", StatusCode::NOT_FOUND),
    ] {
        let response = answer(uri, Some(("x-session-id", "a-session"))).await;
        assert_eq!(response.status(), expected);
        assert!(
            kept_out_of_shared_caches(response.headers()),
            "{uri} answered {expected} and a cache would happily keep that refusal and \
             replay it to a holder whose session is fine"
        );
    }
}

#[tokio::test]
async fn a_redirect_carrying_a_stream_location_is_protected_too() {
    let response = answer("/tracks/soundcloud:tracks:42/stream", None).await;

    assert_eq!(response.status(), StatusCode::FOUND);
    assert!(
        kept_out_of_shared_caches(response.headers()),
        "a 3xx is a cacheable answer, and the location it points at is a private track url"
    );
}

#[tokio::test]
async fn the_answer_that_hands_out_a_session_is_protected_even_though_nothing_was_shown() {
    let response = answer("/auth/callback", None).await;

    assert!(
        kept_out_of_shared_caches(response.headers()),
        "the callback mints the session; storing that answer hands the session to whoever \
         asks next"
    );
}

#[tokio::test]
async fn every_auth_answer_is_protected_without_being_asked() {
    let response = answer("/auth/session", None).await;

    assert!(
        kept_out_of_shared_caches(response.headers()),
        "an anonymous /auth answer still tells a cache what the next holder is about to be given"
    );
}

#[tokio::test]
async fn a_public_answer_nobody_identified_themselves_for_stays_cacheable() {
    let response = answer("/discover/tags", None).await;

    assert!(
        !kept_out_of_shared_caches(response.headers()),
        "if an anonymous public read were locked down too, this whole guard would pass with \
         the capability check deleted and prove nothing"
    );
    assert_eq!(response.headers().get(VARY), None);
}

#[test]
fn the_cors_policy_never_reflects_an_origin_and_trusts_it_at_the_same_time() {
    let router = std::fs::read_to_string(
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/router.rs"),
    )
    .expect("the router is readable");

    let reflects = router.contains("mirror_request()") || router.contains("AllowOrigin::any()");
    let trusts = router.contains("allow_credentials(true)");

    assert!(
        !(reflects && trusts),
        "reflecting whatever origin asked and then allowing credentials makes every browser \
         that has our cookies a confused deputy for any page on the internet; the session \
         lives in `x-session-id` precisely so an origin cannot spend it without knowing it"
    );
    assert!(
        reflects,
        "this guard only means something while the origin is reflected; if the policy became \
         an allow-list, replace it rather than leave a check that can no longer fail"
    );
}

#[test]
fn nothing_in_the_tree_declares_an_answer_publicly_cacheable() {
    let mut declared: Vec<String> = Vec::new();
    let mut stack = vec![std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src")];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            if !name.ends_with(".rs") || name.contains("test") {
                continue;
            }
            let Ok(body) = std::fs::read_to_string(&path) else {
                continue;
            };
            for (number, line) in body.lines().enumerate() {
                if line.contains("\"public") && line.contains("max-age") {
                    declared.push(format!(
                        "{}:{}: {}",
                        path.display(),
                        number + 1,
                        line.trim()
                    ));
                }
            }
        }
    }
    assert!(
        declared.is_empty(),
        "a handler that declares its answer publicly cacheable overrides the protection this \
         layer applies, and it does so for the holder's answer as well:\n{}",
        declared.join("\n")
    );
}
