use std::sync::Arc;
use std::time::Duration;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode, header};
use serde_json::Value;
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;

use crate::common::admission::PublicAdmission;
use crate::config::{AdmissionCfg, AdmissionLimitCfg};
use crate::modules::chat::ChatState;
use crate::rt::rooms::Proxy;
use crate::rt::router::build;
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
        diag_bundle: limit,
        diag_events: limit,
        diag_bytes: limit,
        voice_link_code: limit,
        voice_remote: limit,
        voice_remote_wait: limit,
        voice_page_ip: limit,
        voice_page_client: limit,
        voice_salut_sub: limit,
        voice_utterance: limit,
        voice_resolve: limit,
        voice_alice_ip: limit,
        voice_salut_ip: limit,
        trusted_proxies: Vec::new(),
    };
    PublicAdmission::new(
        crate::redis::connect_admission_url(redis_url, 16).expect("an admission pool"),
        config,
    )
}

pub fn app(chat: Arc<ChatState>, redis_url: &str, rooms_proxy: Option<Arc<Proxy>>) -> axum::Router {
    build(
        RtState {
            pg: chat.pg.clone(),
            auth: chat.auth.clone(),
            admission: admission(redis_url),
            rooms: chat.rooms.clone(),
            chat,
        },
        rooms_proxy,
    )
}

pub async fn session_for(pool: &PgPool, user: &str) -> anyhow::Result<Uuid> {
    session_expiring(pool, user, "1 hour").await
}

pub async fn expired_session_for(pool: &PgPool, user: &str) -> anyhow::Result<Uuid> {
    session_expiring(pool, user, "-1 day").await
}

async fn session_expiring(pool: &PgPool, user: &str, expires_in: &str) -> anyhow::Result<Uuid> {
    let connection = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO soundcloud_connections
            (id, soundcloud_user_id, username, access_token, refresh_token, expires_at, scope)
         VALUES ($1, $2, $2, 'access', 'refresh', now() + $3::text::interval, '')",
    )
    .bind(connection)
    .bind(user)
    .bind(expires_in)
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

pub async fn call(
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
