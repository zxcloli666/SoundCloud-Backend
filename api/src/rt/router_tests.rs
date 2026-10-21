use std::sync::Arc;
use std::time::Duration;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode, header};
use sc_chat_wire::prost::Message;
use sc_chat_wire::proto::InnerResponse;
use sc_chat_wire::seal::ResponseKey;
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;

use super::build;
use crate::common::admission::PublicAdmission;
use crate::config::{AdmissionCfg, AdmissionLimitCfg};
use crate::modules::chat::ChatState;
use crate::modules::chat::test_support::{
    chat_state, hex_field, offline_chat_state, request_vector,
};
use crate::rt::state::RtState;

fn admission(redis_url: &str) -> Arc<PublicAdmission> {
    let limit = AdmissionLimitCfg {
        per_client: 1_000,
        global: 1_000_000,
    };
    let config = AdmissionCfg {
        window: Duration::from_secs(60),
        timeout: Duration::from_millis(500),
        max_in_flight: 16,
        login: limit,
        link_create: limit,
        resolve: limit,
        sc_search: limit,
        catalog_miss: limit,
        room_list: limit,
        room_create: limit,
        room_join: limit,
    };
    PublicAdmission::new(
        crate::redis::connect_admission_url(redis_url, 16).expect("an admission pool"),
        config,
    )
}

fn app(chat: Arc<ChatState>, redis_url: &str) -> axum::Router {
    build(RtState {
        pg: chat.pg.clone(),
        auth: chat.auth.clone(),
        admission: admission(redis_url),
        rooms: chat.rooms.clone(),
        chat,
    })
}

fn offline_app(chat_enabled: bool) -> axum::Router {
    app(offline_chat_state(chat_enabled), "redis://127.0.0.1:1")
}

async fn post_envelope(app: &axum::Router, envelope: Vec<u8>) -> axum::response::Response {
    app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/chat/x")
                .header(header::CONTENT_TYPE, "application/octet-stream")
                .body(Body::from(envelope))
                .expect("a request"),
        )
        .await
        .expect("an answer")
}

async fn sealed_answer(app: &axum::Router, case: &Value) -> (InnerResponse, Value) {
    let response = post_envelope(app, hex_field(case, "envelope")).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers()[header::CONTENT_TYPE],
        "application/octet-stream"
    );
    assert_eq!(
        response.headers()[header::CACHE_CONTROL],
        "private, no-store"
    );
    let sealed = to_bytes(response.into_body(), 64 * 1024)
        .await
        .expect("a body");
    let key = ResponseKey::from_bytes(hex_field(case, "response_key").try_into().expect("32"));
    let enc: [u8; 32] = hex_field(case, "enc").try_into().expect("32");
    let plain = key
        .open(&enc, &sealed)
        .expect("the answer opens with the request key");
    let inner = InnerResponse::decode(plain.as_slice()).expect("an inner response");
    let body = serde_json::from_slice(&inner.body).expect("a json body");
    (inner, body)
}

#[tokio::test]
async fn health_answers_on_rt() {
    let response = offline_app(true)
        .oneshot(
            Request::get("/health")
                .body(Body::empty())
                .expect("a request"),
        )
        .await
        .expect("an answer");
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn a_vector_envelope_gets_a_sealed_chat_disabled_while_chat_is_off() {
    let (inner, body) = sealed_answer(&offline_app(false), &request_vector("sync")).await;
    assert_eq!(inner.status, 503);
    assert_eq!(body, json!({"error": "chat_disabled"}));
    assert!(
        inner
            .headers
            .iter()
            .any(|h| h.name == "content-type" && h.value == "application/json")
    );
}

#[tokio::test]
async fn sealed_login_is_not_switched_off_with_chat() {
    let (inner, body) = sealed_answer(&offline_app(false), &request_vector("login")).await;
    assert_eq!(inner.status, 401);
    assert_eq!(body["error"], "chat_clock");
}

#[tokio::test]
async fn with_chat_on_a_recorded_vector_is_too_old_and_gets_a_sealed_chat_clock() {
    for name in ["sync", "history", "login"] {
        let (inner, body) = sealed_answer(&offline_app(true), &request_vector(name)).await;
        assert_eq!(inner.status, 401, "{name}");
        assert_eq!(body["error"], "chat_clock", "{name}");
        assert!(body["server_now"].as_i64().is_some(), "{name}");
    }
}

#[tokio::test]
async fn a_broken_envelope_is_a_plain_400() {
    let app = offline_app(true);
    let envelope = hex_field(&request_vector("sync"), "envelope");
    let mut unknown_key = envelope.clone();
    unknown_key[1] = 9;
    let mut tampered = envelope.clone();
    let last = tampered.len() - 1;
    tampered[last] ^= 1;
    let mut new_version = envelope.clone();
    new_version[0] = 2;
    for broken in [
        Vec::new(),
        envelope[..20].to_vec(),
        unknown_key,
        tampered,
        new_version,
    ] {
        let response = post_envelope(&app, broken).await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }
}

#[tokio::test]
async fn an_envelope_over_64_kb_is_refused_before_any_crypto() {
    let response = post_envelope(&offline_app(true), vec![1; 64 * 1024 + 1]).await;
    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
}

#[tokio::test]
async fn the_full_api_routes_are_not_on_rt() {
    let app = offline_app(true);
    for uri in ["/me", "/tracks?ids=1", "/admin/metrics", "/auth/login"] {
        let response = app
            .clone()
            .oneshot(Request::get(uri).body(Body::empty()).expect("a request"))
            .await
            .expect("an answer");
        assert_eq!(response.status(), StatusCode::NOT_FOUND, "{uri}");
    }
}

fn redis_url() -> String {
    std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".to_owned())
}

async fn session_for(pool: &PgPool, user: &str) -> anyhow::Result<Uuid> {
    let connection = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO soundcloud_connections
            (id, soundcloud_user_id, username, access_token, refresh_token, expires_at, scope)
         VALUES ($1, $2, $2, 'access', 'refresh', now() + interval '1 hour', '')",
    )
    .bind(connection)
    .bind(user)
    .execute(pool)
    .await?;
    let session = Uuid::now_v7();
    sqlx::query("INSERT INTO sessions (id, soundcloud_connection_id) VALUES ($1, $2)")
        .bind(session)
        .bind(connection)
        .execute(pool)
        .await?;
    Ok(session)
}

async fn call(
    app: &axum::Router,
    session: Uuid,
    method: &str,
    uri: &str,
    body: Option<Value>,
) -> anyhow::Result<(StatusCode, Value)> {
    let mut request = Request::builder()
        .method(method)
        .uri(uri)
        .header("x-session-id", session.to_string());
    if body.is_some() {
        request = request.header(header::CONTENT_TYPE, "application/json");
    }
    let body = body.map_or_else(Body::empty, |body| Body::from(body.to_string()));
    let response = app.clone().oneshot(request.body(body)?).await?;
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 1024 * 1024).await?;
    Ok((
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    ))
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Redis"]
async fn plain_rooms_run_on_rt_with_the_generic_session(pool: PgPool) -> anyhow::Result<()> {
    let redis = crate::redis::connect_url(&redis_url())?;
    let app = app(chat_state(pool.clone(), redis, true), &redis_url());
    let host = session_for(&pool, "9101").await?;
    let guest = session_for(&pool, "9202").await?;

    let (status, room) = call(&app, host, "POST", "/rooms", Some(json!({"name": "Host"}))).await?;
    assert_eq!(status, StatusCode::CREATED, "{room}");
    let code = room["code"].as_str().unwrap_or_default().to_owned();
    assert_eq!(room["hostId"], "9101");

    let (status, room) = call(
        &app,
        guest,
        "POST",
        &format!("/rooms/{code}/members"),
        Some(json!({"name": "Guest"})),
    )
    .await?;
    assert_eq!(status, StatusCode::OK, "{room}");
    assert_eq!(room["members"].as_array().map(Vec::len), Some(2));

    let (status, listed) = call(&app, guest, "GET", "/rooms/public", None).await?;
    assert_eq!(status, StatusCode::OK, "{listed}");

    let (status, _) = call(
        &app,
        guest,
        "DELETE",
        &format!("/rooms/{code}/members/me"),
        None,
    )
    .await?;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _) = call(
        &app,
        host,
        "DELETE",
        &format!("/rooms/{code}/members/me"),
        None,
    )
    .await?;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (status, _) = call(&app, Uuid::now_v7(), "GET", "/rooms/public", None).await?;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    Ok(())
}
