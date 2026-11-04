use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::extract::{ConnectInfo, Request};
use axum::http::header::{self, HeaderMap, HeaderValue};
use axum::http::{Method, StatusCode};
use axum::response::IntoResponse;
use serde_json::{Value, json};
use tower::ServiceExt;
use url::Url;

use super::{Proxy, climbs, router, upstream_headers};

async fn serve(app: Router) -> Url {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("a free port");
    let address = listener.local_addr().expect("an address");
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("a server");
    });
    Url::parse(&format!("http://{address}")).expect("a url")
}

async fn echo(request: Request) -> impl IntoResponse {
    let (parts, body) = request.into_parts();
    let body = to_bytes(body, 1 << 20).await.expect("a body");
    let header = |name: &str| {
        parts
            .headers
            .get(name)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned)
    };
    let seen = json!({
        "method": parts.method.as_str(),
        "uri": parts.uri.to_string(),
        "session": header("x-session-id"),
        "forwarded": header("x-forwarded-for"),
        "type": header("content-type"),
        "connection": header("connection"),
        "body": String::from_utf8_lossy(&body),
    });
    (
        StatusCode::CREATED,
        [("x-room-version", "7"), ("cache-control", "no-cache")],
        axum::Json(seen),
    )
}

async fn seen(response: axum::response::Response) -> Value {
    let bytes = to_bytes(response.into_body(), 1 << 20)
        .await
        .expect("a body");
    serde_json::from_slice(&bytes).expect("json")
}

#[tokio::test]
async fn rooms_go_upstream_verbatim_and_the_answer_comes_back_whole() {
    let api = serve(Router::new().fallback(echo)).await;
    let app = router::<()>(Proxy::new(&api));
    let request = Request::builder()
        .method(Method::PUT)
        .uri("/rooms/ABC123/playback?since=5&follow=true")
        .header("x-session-id", "9f0c")
        .header("x-forwarded-for", "203.0.113.9")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(r#"{"status":"paused"}"#))
        .expect("a request");

    let response = app
        .clone()
        .oneshot({
            let mut request = request;
            request.extensions_mut().insert(ConnectInfo(
                "172.30.7.1:5555".parse::<SocketAddr>().expect("peer"),
            ));
            request
        })
        .await
        .expect("an answer");

    assert_eq!(response.status(), StatusCode::CREATED);
    assert_eq!(response.headers()["x-room-version"], "7");
    assert_eq!(response.headers()["cache-control"], "no-cache");
    let body = seen(response).await;
    assert_eq!(body["method"], "PUT");
    assert_eq!(body["uri"], "/rooms/ABC123/playback?since=5&follow=true");
    assert_eq!(body["session"], "9f0c");
    assert_eq!(body["forwarded"], "203.0.113.9");
    assert_eq!(body["type"], "application/json");
    assert_eq!(body["body"], r#"{"status":"paused"}"#);
}

#[tokio::test]
async fn without_a_forwarded_header_the_peer_becomes_the_client() {
    let api = serve(Router::new().fallback(echo)).await;
    let app = router::<()>(Proxy::new(&api));
    let mut request = Request::post("/rooms")
        .body(Body::from("{}"))
        .expect("a request");
    request.extensions_mut().insert(ConnectInfo(
        "198.51.100.4:40000".parse::<SocketAddr>().expect("peer"),
    ));

    let response = app.oneshot(request).await.expect("an answer");

    let body = seen(response).await;
    assert_eq!(body["method"], "POST");
    assert_eq!(body["uri"], "/rooms");
    assert_eq!(body["forwarded"], "198.51.100.4");
}

#[tokio::test]
async fn only_rooms_paths_are_forwarded() {
    let api = serve(Router::new().fallback(echo)).await;
    let app = router::<()>(Proxy::new(&api));
    for uri in ["/me", "/roomsx", "/admin/rooms/handoff"] {
        let response = app
            .clone()
            .oneshot(Request::get(uri).body(Body::empty()).expect("a request"))
            .await
            .expect("an answer");
        assert_eq!(response.status(), StatusCode::NOT_FOUND, "{uri}");
    }
}

#[tokio::test]
async fn a_slow_upstream_is_a_504_and_a_dead_one_a_502() {
    let slow = serve(Router::new().fallback(|| async {
        tokio::time::sleep(Duration::from_secs(5)).await;
        StatusCode::OK
    }))
    .await;
    let app = router::<()>(Proxy::with_timeout(&slow, Duration::from_millis(200)));
    let response = app
        .oneshot(
            Request::get("/rooms/public")
                .body(Body::empty())
                .expect("a request"),
        )
        .await
        .expect("an answer");
    assert_eq!(response.status(), StatusCode::GATEWAY_TIMEOUT);
    assert_eq!(seen(response).await, json!({"error": "rooms_timeout"}));

    let dead = Url::parse("http://127.0.0.1:1").expect("a url");
    let app = router::<()>(Proxy::new(&dead));
    let response = app
        .oneshot(
            Request::get("/rooms/public")
                .body(Body::empty())
                .expect("a request"),
        )
        .await
        .expect("an answer");
    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    assert_eq!(seen(response).await, json!({"error": "rooms_unavailable"}));
}

#[test]
fn hop_by_hop_headers_and_the_host_are_not_forwarded() {
    let mut incoming = HeaderMap::new();
    for (name, value) in [
        ("host", "chat.scnative.dev"),
        ("connection", "keep-alive"),
        ("transfer-encoding", "chunked"),
        ("content-length", "12"),
        ("x-session-id", "abc"),
        ("accept-encoding", "gzip"),
    ] {
        incoming.append(name, HeaderValue::from_static(value));
    }

    let sent = upstream_headers(&incoming, None);

    let names: Vec<&str> = sent.keys().map(|name| name.as_str()).collect();
    assert_eq!(names.len(), 2, "{names:?}");
    assert_eq!(sent["x-session-id"], "abc");
    assert_eq!(sent["accept-encoding"], "gzip");
}

#[tokio::test]
async fn a_path_that_climbs_is_refused_before_it_is_forwarded() {
    let api = serve(Router::new().fallback(echo)).await;
    let app = router::<()>(Proxy::new(&api));
    for uri in [
        "/rooms/../admin/rooms/handoff",
        "/rooms/%2e%2e/admin",
        "/rooms/%2E%2E/admin",
        "/rooms/.%2e/admin",
        "/rooms/%2e./admin",
        "/rooms/ABC/%2E%2e",
    ] {
        let response = app
            .clone()
            .oneshot(Request::get(uri).body(Body::empty()).expect("a request"))
            .await
            .expect("an answer");
        assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{uri}");
        assert_eq!(seen(response).await["error"], "rooms_bad_path", "{uri}");
    }
}

#[test]
fn only_a_whole_dot_dot_segment_climbs() {
    assert!(climbs("/rooms/../x"));
    assert!(climbs("/rooms/%2e%2E"));
    assert!(!climbs("/rooms/a..b"));
    assert!(!climbs("/rooms/ABC123/members"));
    assert!(!climbs("/rooms/%2e/x"));
}

#[tokio::test]
async fn hundreds_of_parked_requests_all_complete_through_one_proxy() {
    let parked_now = Arc::new(AtomicUsize::new(0));
    let peak = Arc::new(AtomicUsize::new(0));
    let upstream = {
        let (parked_now, peak) = (parked_now.clone(), peak.clone());
        Router::new().fallback(move || {
            let (parked_now, peak) = (parked_now.clone(), peak.clone());
            async move {
                let now = parked_now.fetch_add(1, Ordering::SeqCst) + 1;
                peak.fetch_max(now, Ordering::SeqCst);
                tokio::time::sleep(Duration::from_millis(1_500)).await;
                parked_now.fetch_sub(1, Ordering::SeqCst);
                StatusCode::OK
            }
        })
    };
    let api = serve(upstream).await;
    let app = router::<()>(Proxy::new(&api));

    let mut parked = tokio::task::JoinSet::new();
    for member in 0..300 {
        if member % 25 == 0 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let app = app.clone();
        parked.spawn(async move {
            app.oneshot(
                Request::get(format!("/rooms/ABC123?member={member}"))
                    .body(Body::empty())
                    .expect("a request"),
            )
            .await
            .expect("an answer")
            .status()
        });
    }
    let mut done = 0;
    while let Some(status) = parked.join_next().await {
        assert_eq!(status.expect("a task"), StatusCode::OK);
        done += 1;
    }

    assert_eq!(done, 300);
    assert_eq!(peak.load(Ordering::SeqCst), 300);
}
