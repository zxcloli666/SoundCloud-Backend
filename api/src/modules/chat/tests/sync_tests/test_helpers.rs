use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use axum::body::{Body, to_bytes};
use axum::http::{HeaderMap, Request, StatusCode, header};
use chrono::Utc;
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;

use crate::modules::chat::ChatState;
use crate::modules::chat::caller::{CallerDevice, ChatCaller};
use crate::modules::chat::model::Tier;
use crate::modules::chat::routes;
use crate::modules::chat::sync::changes;
use crate::modules::chat::sync::prefs::Prefs;
use crate::modules::chat::test_support::chat_state;

static NEXT_USER: AtomicU64 = AtomicU64::new(0);

pub struct Answer {
    pub status: StatusCode,
    pub headers: HeaderMap,
    pub body: Value,
}

pub fn redis_url() -> String {
    std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".to_owned())
}

pub fn redis_db(index: u8) -> String {
    let mut url = url::Url::parse(&redis_url()).expect("a redis url");
    url.set_path(&format!("/{index}"));
    url.to_string()
}

pub fn redis_pool(url: &str) -> deadpool_redis::Pool {
    crate::redis::connect_url(url).expect("a redis pool")
}

pub fn dead_pg() -> PgPool {
    sqlx::postgres::PgPoolOptions::new()
        .acquire_timeout(std::time::Duration::from_millis(200))
        .connect_lazy("postgres://scd@127.0.0.1:1/none")
        .expect("a lazy pool")
}

pub fn without_postgres() -> Arc<ChatState> {
    chat_state(dead_pg(), redis_pool(&redis_url()), true)
}

pub fn with_postgres(pool: &PgPool) -> Arc<ChatState> {
    chat_state(pool.clone(), redis_pool(&redis_url()), true)
}

pub fn with_max_parked(chat: Arc<ChatState>, max_parked: usize) -> Arc<ChatState> {
    let mut chat = chat;
    Arc::get_mut(&mut chat)
        .expect("a fresh state")
        .config
        .max_parked = max_parked;
    chat
}

pub fn warm(chat: &ChatState, users: &[&str]) -> Uuid {
    let incarnation = Uuid::now_v7();
    chat.hub
        .incarnation
        .set(incarnation)
        .expect("a fresh incarnation");
    for user in users {
        chat.hub.prefs.remember(user, Prefs::default());
    }
    incarnation
}

pub fn user() -> String {
    let base = u64::try_from(Utc::now().timestamp_micros()).unwrap_or(0) % 1_000_000_000;
    format!(
        "77{base}{:04}",
        NEXT_USER.fetch_add(1, Ordering::Relaxed) % 10_000
    )
}

pub fn caller(user_id: &str, device: Uuid) -> ChatCaller {
    caller_on(user_id, device, Tier::Direct)
}

pub fn caller_on(user_id: &str, device: Uuid, tier: Tier) -> ChatCaller {
    ChatCaller {
        user_id: user_id.to_owned(),
        session_id: Uuid::now_v7(),
        device: Some(CallerDevice {
            id: device,
            created_at: Utc::now(),
            history_unlocked: true,
        }),
        tier,
        max_bytes: 262_144,
    }
}

pub async fn call(
    chat: &Arc<ChatState>,
    caller: ChatCaller,
    method: &str,
    uri: &str,
    body: Option<Value>,
) -> Answer {
    let router = routes::inner().with_state(chat.clone());
    let mut request = Request::builder().method(method).uri(uri);
    if body.is_some() {
        request = request.header(header::CONTENT_TYPE, "application/json");
    }
    let mut request = request
        .body(body.map_or_else(Body::empty, |body| Body::from(body.to_string())))
        .expect("a request");
    request.extensions_mut().insert(caller);
    let response = router.oneshot(request).await.expect("an answer");
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = to_bytes(response.into_body(), 1024 * 1024)
        .await
        .expect("a body");
    Answer {
        status,
        headers,
        body: serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    }
}

pub async fn sync(chat: &Arc<ChatState>, caller: ChatCaller, body: Value) -> Answer {
    call(chat, caller, "POST", "/chat/sync", Some(body)).await
}

pub fn poll(pos: u64, inc: Uuid, hold_ms: u64) -> Value {
    json!({"pos": pos, "inc": inc, "hold_ms": hold_ms, "max_bytes": 262_144})
}

pub async fn position(chat: &ChatState, user_id: &str) -> u64 {
    changes::read(chat, user_id, 0)
        .await
        .expect("versions")
        .versions
        .version
}

pub async fn conversation(pool: &PgPool, kind: &str, members: &[&str]) -> Uuid {
    let id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO chat.conversations (id, kind, created_by, member_count) VALUES ($1, $2, $3, $4)",
    )
    .bind(id)
    .bind(kind)
    .bind(members.first().copied().unwrap_or("0"))
    .bind(i32::try_from(members.len()).unwrap_or(0))
    .execute(pool)
    .await
    .expect("a conversation");
    for member in members {
        member_in(pool, id, member, "active").await;
    }
    id
}

pub async fn member_in(pool: &PgPool, conversation: Uuid, user_id: &str, state: &str) {
    sqlx::query(
        "INSERT INTO chat.members (conversation_id, user_id, state, accepted) VALUES ($1, $2, $3, true)
         ON CONFLICT (conversation_id, user_id) DO UPDATE SET state = EXCLUDED.state",
    )
    .bind(conversation)
    .bind(user_id)
    .bind(state)
    .execute(pool)
    .await
    .expect("a member");
}

pub async fn messages(
    pool: &PgPool,
    conversation: Uuid,
    sender: &str,
    count: i64,
    body_len: usize,
) {
    for _ in 0..count {
        sqlx::query(
            "WITH bumped AS (
                UPDATE chat.conversations SET last_seq = last_seq + 1 WHERE id = $1 RETURNING last_seq
             )
             INSERT INTO chat.messages (conversation_id, seq, client_id, sender_user, class, epoch, body)
             SELECT $1, last_seq, $2, $3, 'post', 0, $4 FROM bumped",
        )
        .bind(conversation)
        .bind(Uuid::new_v4())
        .bind(sender)
        .bind(vec![7_u8; body_len])
        .execute(pool)
        .await
        .expect("a message");
    }
}

pub async fn device(pool: &PgPool, user_id: &str, status: &str) -> Uuid {
    let id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO chat.devices (id, user_id, session_id, status, auth_pub, box_pub, platform, name, app_version, tier)
         VALUES ($1, $2, $3, $4, '\\x01', '\\x02', 'macos', 'Mac', '9.0.0', 'direct')",
    )
    .bind(id)
    .bind(user_id)
    .bind(Uuid::now_v7())
    .bind(status)
    .execute(pool)
    .await
    .expect("a device");
    id
}

pub async fn revoke(pool: &PgPool, device: Uuid) {
    sqlx::query("UPDATE chat.devices SET status = 'revoked', revoked_at = now() WHERE id = $1")
        .bind(device)
        .execute(pool)
        .await
        .expect("a revoke");
}

pub async fn incarnation(pool: &PgPool) -> Uuid {
    sqlx::query_scalar("SELECT incarnation FROM chat.meta WHERE id = 1")
        .fetch_one(pool)
        .await
        .expect("an incarnation")
}

pub async fn settle(chat: &ChatState, parked: usize) {
    for _ in 0..400 {
        if chat.hub.parked() >= parked {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
    panic!("only {} of {parked} polls parked", chat.hub.parked());
}
