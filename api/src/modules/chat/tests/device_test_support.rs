use std::sync::Arc;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use sc_chat_wire::bind::make_bind;
use sc_chat_wire::prost::Message;
use serde_json::Value;
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;

use crate::modules::chat::caller::CallerDevice;
use crate::modules::chat::model::{DeviceRow, Tier};
use crate::modules::chat::test_support::chat_state;
use crate::modules::chat::{ChatCaller, ChatState, devices, routes, rt_keys};

pub struct DeviceKeys {
    pub id: Uuid,
    pub auth_seed: [u8; 32],
    pub box_secret: [u8; 32],
    pub box_pub: [u8; 32],
}

impl DeviceKeys {
    pub fn new() -> Self {
        let box_secret: [u8; 32] = sc_chat_wire::random_bytes().expect("randomness");
        Self {
            id: Uuid::new_v4(),
            auth_seed: sc_chat_wire::random_bytes().expect("randomness"),
            box_pub: sc_chat_wire::ck::box_public(&box_secret).expect("a box key"),
            box_secret,
        }
    }

    pub fn bind(&self) -> Vec<u8> {
        make_bind(&self.id.to_string(), &self.auth_seed, &self.box_pub, 1).encode_to_vec()
    }
}

pub struct Enrolled {
    pub keys: DeviceKeys,
    pub session: Uuid,
    pub row: DeviceRow,
}

pub fn redis() -> deadpool_redis::Pool {
    let url = std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".to_owned());
    crate::redis::connect_url(&url).expect("a redis pool")
}

pub fn state(pool: &PgPool) -> Arc<ChatState> {
    chat_state(pool.clone(), redis(), true)
}

pub fn fresh_user() -> String {
    format!("9{:010}", rand::random::<u32>())
}

pub async fn fresh_device_budget(chat: &ChatState, user: &str) {
    let mut conn = chat.redis.get().await.expect("a redis connection");
    let _: i64 = redis::cmd("DEL")
        .arg(rt_keys::limit("device:1d", user))
        .query_async(&mut conn)
        .await
        .expect("a reset device budget");
}

pub async fn session(pool: &PgPool, user: &str, bind: Option<Vec<u8>>) -> Uuid {
    let connection = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO soundcloud_connections
            (id, soundcloud_user_id, username, access_token, refresh_token, expires_at, scope)
         VALUES ($1, $2, $2, 'access', 'refresh', now() + interval '1 hour', '')",
    )
    .bind(connection)
    .bind(user)
    .execute(pool)
    .await
    .expect("a connection");
    let session = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO sessions (id, soundcloud_connection_id, chat_bind) VALUES ($1, $2, $3)",
    )
    .bind(session)
    .bind(connection)
    .bind(bind)
    .execute(pool)
    .await
    .expect("a session");
    session
}

pub fn caller(user: &str, session: Uuid, device: Option<&DeviceRow>) -> ChatCaller {
    ChatCaller {
        user_id: user.to_owned(),
        session_id: session,
        device: device.map(CallerDevice::from),
        tier: Tier::Relay,
        max_bytes: 262_144,
    }
}

pub async fn call(
    chat: &Arc<ChatState>,
    caller: ChatCaller,
    method: &str,
    uri: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut request = Request::builder()
        .method(method)
        .uri(uri)
        .header("content-type", "application/json")
        .body(body.map_or_else(Body::empty, |body| Body::from(body.to_string())))
        .expect("a request");
    request.extensions_mut().insert(caller);
    let response = routes::inner()
        .with_state(chat.clone())
        .oneshot(request)
        .await
        .expect("an answer");
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 1024 * 1024)
        .await
        .expect("a body");
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

pub fn desktop() -> Value {
    serde_json::json!({
        "platform": "macos",
        "name": "MacBook",
        "app_version": "8.7.0",
        "caps": ["embeds"]
    })
}

pub async fn enroll(chat: &Arc<ChatState>, pool: &PgPool, user: &str) -> Enrolled {
    fresh_device_budget(chat, user).await;
    let keys = DeviceKeys::new();
    let session = session(pool, user, Some(keys.bind())).await;
    let (status, answer) = call(
        chat,
        caller(user, session, None),
        "POST",
        "/chat/devices",
        Some(desktop()),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    let row = devices::lookup(chat, keys.id)
        .await
        .expect("a lookup")
        .expect("a device");
    Enrolled { keys, session, row }
}

pub async fn age(
    chat: &Arc<ChatState>,
    pool: &PgPool,
    device: &DeviceRow,
    hours: i64,
) -> DeviceRow {
    sqlx::query(
        "UPDATE chat.devices SET created_at = now() - $2::int * interval '1 hour' WHERE id = $1",
    )
    .bind(device.id)
    .bind(i32::try_from(hours).expect("hours fit"))
    .execute(pool)
    .await
    .expect("an aged device");
    chat.devices.forget(&device.user_id, &[device.id]);
    devices::lookup(chat, device.id)
        .await
        .expect("a lookup")
        .expect("a device")
}

pub async fn conversation(pool: &PgPool, kind: &str, members: &[&str]) -> Uuid {
    let id = Uuid::now_v7();
    sqlx::query("INSERT INTO chat.conversations (id, kind, created_by) VALUES ($1, $2, $3)")
        .bind(id)
        .bind(kind)
        .bind(members[0])
        .execute(pool)
        .await
        .expect("a conversation");
    for member in members {
        sqlx::query(
            "INSERT INTO chat.members (conversation_id, user_id, state) VALUES ($1, $2, 'active')",
        )
        .bind(id)
        .bind(*member)
        .execute(pool)
        .await
        .expect("a member");
    }
    id
}

pub async fn rekey_pending(pool: &PgPool, conversation: Uuid) -> bool {
    sqlx::query_scalar("SELECT rekey FROM chat.conversations WHERE id = $1")
        .bind(conversation)
        .fetch_one(pool)
        .await
        .expect("a conversation")
}
