use std::sync::Arc;

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode, header};
use chat_wire::prost::Message;
use chat_wire::proto::{Inner, InnerResponse, Tier};
use chat_wire::seal::{RequestSealer, SealedRequest};
use chat_wire::sign::sign_request;
use chrono::Utc;
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;

use crate::modules::chat::moderation::testing::{ADMIN_TOKEN, state_with};
use crate::modules::chat::test_support::vector_ock;
use crate::modules::chat::{ChatState, b64, routes};

pub struct Device {
    pub user: String,
    pub id: Uuid,
    pub session: Uuid,
    pub auth_seed: [u8; 32],
    pub box_secret: [u8; 32],
}

impl Device {
    pub fn from_seeded(seeded: &Value) -> Self {
        let bytes = |field: &str| -> [u8; 32] {
            let raw = b64::decode(seeded[field].as_str().expect("a base64 field")).expect("base64");
            raw.try_into().expect("32 bytes")
        };
        let id =
            |field: &str| Uuid::parse_str(seeded[field].as_str().expect("an id")).expect("a uuid");
        Self {
            user: seeded["user"].as_str().expect("a user").to_owned(),
            id: id("id"),
            session: id("session"),
            auth_seed: bytes("auth_seed"),
            box_secret: bytes("box_secret"),
        }
    }
}

pub fn redis_url(db: u8) -> String {
    let base = std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".to_owned());
    let mut url = url::Url::parse(&base).expect("a redis url");
    url.set_path(&format!("/{db}"));
    url.to_string()
}

pub fn redis(db: u8) -> deadpool_redis::Pool {
    crate::redis::connect_url(&redis_url(db)).expect("a redis pool")
}

pub async fn flushed(db: u8) -> deadpool_redis::Pool {
    let pool = redis(db);
    let mut conn = pool.get().await.expect("a redis connection");
    let _: () = redis::cmd("FLUSHDB")
        .query_async(&mut conn)
        .await
        .expect("a flushed db");
    drop(conn);
    pool
}

pub fn state(pg: PgPool, redis: deadpool_redis::Pool, loadtest: bool) -> Arc<ChatState> {
    let mut chat = Arc::into_inner(state_with(pg, redis, None)).expect("a fresh state");
    chat.config.loadtest = loadtest;
    Arc::new(chat)
}

pub async fn admin(chat: &Arc<ChatState>, path: &str, body: Option<Value>) -> (StatusCode, Value) {
    let request = Request::post(path)
        .header("x-chat-admin-token", ADMIN_TOKEN)
        .header("x-chat-admin-user", "owner")
        .header(header::CONTENT_TYPE, "application/json")
        .body(body.map_or_else(Body::empty, |body| Body::from(body.to_string())))
        .expect("a request");
    let app: Router = routes::public(chat.clone());
    let response = app.oneshot(request).await.expect("an answer");
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 16 * 1024 * 1024)
        .await
        .expect("a body");
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

pub async fn seed(chat: &Arc<ChatState>, users: u32, per_user: u32, groups: u32) -> Value {
    let body = json!({"users": users, "devices_per_user": per_user, "groups": groups});
    let (status, seeded) = admin(chat, "/chat-admin/loadtest/seed", Some(body)).await;
    assert_eq!(status, StatusCode::OK, "{seeded}");
    seeded
}

pub fn now_ms() -> u64 {
    u64::try_from(Utc::now().timestamp_millis()).expect("a positive clock")
}

pub fn sealed(device: &Device, method: &str, path: &str, body: Option<Value>) -> SealedRequest {
    let mut inner = Inner {
        ts_ms: now_ms(),
        session_id: Some(device.session.to_string()),
        device_id: Some(device.id.to_string()),
        method: method.to_owned(),
        path: path.to_owned(),
        body: body
            .map(|body| body.to_string().into_bytes())
            .unwrap_or_default(),
        tier: Tier::Relay as i32,
        max_bytes: 262_144,
        ..Inner::default()
    };
    let ikm: [u8; 32] = chat_wire::random_bytes().expect("randomness");
    let sealer = RequestSealer::new(1, &vector_ock().public(), ikm).expect("a sealer");
    sign_request(&device.auth_seed, &mut inner, sealer.enc());
    sealer
        .seal(&inner.encode_to_vec())
        .expect("a sealed request")
}

pub struct Answer {
    pub status: u32,
    pub body: Value,
}

pub async fn call(
    chat: &Arc<ChatState>,
    device: &Device,
    method: &str,
    path: &str,
    body: Option<Value>,
) -> Answer {
    let request = sealed(device, method, path, body);
    let app: Router = routes::public(chat.clone());
    let response = app
        .oneshot(
            Request::post("/chat/x")
                .header(header::CONTENT_TYPE, "application/octet-stream")
                .body(Body::from(request.envelope.clone()))
                .expect("a request"),
        )
        .await
        .expect("an answer");
    assert_eq!(response.status(), StatusCode::OK);
    let sealed = to_bytes(response.into_body(), 1024 * 1024)
        .await
        .expect("a body");
    let plain = request
        .response_key
        .open(&request.enc, &sealed)
        .expect("the answer opens");
    let inner = InnerResponse::decode(plain.as_slice()).expect("an inner answer");
    Answer {
        status: inner.status,
        body: serde_json::from_slice(&inner.body).unwrap_or(Value::Null),
    }
}

pub async fn count(pg: &PgPool, sql: &str) -> i64 {
    sqlx::query_scalar(sql)
        .fetch_one(pg)
        .await
        .expect("a count")
}

pub async fn redis_keys(pool: &deadpool_redis::Pool, pattern: &str) -> Vec<String> {
    let mut conn = pool.get().await.expect("a redis connection");
    redis::cmd("KEYS")
        .arg(pattern)
        .query_async(&mut conn)
        .await
        .expect("keys")
}
