use std::sync::Arc;

use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

use crate::modules::chat::devices::test_support::{call as device_call, caller, enroll};
use crate::modules::chat::media::support::{bench as media_bench, call};
use crate::modules::chat::public_chat::ID;
use crate::modules::chat::{ChatState, rt_keys};

pub use crate::modules::chat::media::support::{error, user};

pub const OWNER: &str = "1275950656";

static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

pub async fn serial() -> tokio::sync::MutexGuard<'static, ()> {
    SERIAL.lock().await
}

pub fn bench(pool: &PgPool) -> Arc<ChatState> {
    let mut chat = media_bench(pool);
    Arc::get_mut(&mut chat)
        .expect("a fresh state")
        .config
        .public = true;
    chat
}

pub fn closed(pool: &PgPool) -> Arc<ChatState> {
    media_bench(pool)
}

pub async fn reset(chat: &ChatState) {
    let mut conn = chat.redis.get().await.expect("a redis connection");
    let mut keys: Vec<String> = redis::cmd("KEYS")
        .arg(format!("chat:lim:*:{OWNER}"))
        .query_async(&mut conn)
        .await
        .expect("the owner's limits");
    keys.extend([
        rt_keys::PUBLIC_WATCHERS.to_owned(),
        rt_keys::PUBLIC_GATE.to_owned(),
        rt_keys::PUBLIC_TRAIL.to_owned(),
    ]);
    let _: i64 = redis::cmd("DEL")
        .arg(&keys)
        .query_async(&mut conn)
        .await
        .expect("cleared keys");
}

pub async fn keys_status(chat: &Arc<ChatState>, pool: &PgPool, user: &str) -> (u16, Value) {
    let device = enroll(chat, pool, user).await;
    let mut row = device.row.clone();
    row.caps
        .push(crate::modules::chat::devices::CAP_PUBLIC_CHAT.to_owned());
    let (status, body) = device_call(
        chat,
        caller(user, device.session, Some(&row)),
        "GET",
        &format!("/chat/conversations/{ID}/keys"),
        None,
    )
    .await;
    (status.as_u16(), body)
}

pub async fn ask(
    chat: &Arc<ChatState>,
    user: &str,
    method: &str,
    uri: &str,
    body: Option<Value>,
) -> (u16, Value) {
    call(chat, user, method, uri, body).await
}

pub async fn open(chat: &Arc<ChatState>, user: &str) -> Value {
    let answer = ask(chat, user, "POST", "/chat/public/open", None).await;
    assert_eq!(answer.0, 200, "{answer:?}");
    answer.1
}

pub fn post_body(media: &[Uuid]) -> Value {
    json!({
        "client_id": Uuid::now_v7(),
        "class": "post",
        "epoch": 0,
        "body": "AQID",
        "media": media
    })
}

pub async fn post(chat: &Arc<ChatState>, user: &str) -> (u16, Value) {
    send(chat, user, post_body(&[])).await
}

pub async fn send(chat: &Arc<ChatState>, user: &str, body: Value) -> (u16, Value) {
    ask(
        chat,
        user,
        "POST",
        &format!("/chat/conversations/{ID}/messages"),
        Some(body),
    )
    .await
}

pub async fn posted(chat: &Arc<ChatState>, user: &str) -> i64 {
    let answer = post(chat, user).await;
    assert_eq!(answer.0, 200, "{answer:?}");
    answer.1["seq"].as_i64().expect("a seq")
}

pub async fn watch(chat: &Arc<ChatState>, user: &str) {
    let answer = ask(chat, user, "POST", "/chat/public/watch", None).await;
    assert_eq!(answer.0, 204, "{answer:?}");
}

pub async fn moderator(chat: &Arc<ChatState>, user: &str) {
    let answer = ask(
        chat,
        OWNER,
        "PUT",
        &format!("/chat/public/moderators/{user}"),
        Some(json!({"on": true})),
    )
    .await;
    assert_eq!(answer.0, 204, "{answer:?}");
}

pub async fn version_of(chat: &ChatState, user: &str) -> Option<u64> {
    let mut conn = chat.redis.get().await.expect("a redis connection");
    redis::cmd("HGET")
        .arg(rt_keys::version(user))
        .arg("v")
        .query_async(&mut conn)
        .await
        .expect("a version")
}

pub async fn audit_count(pool: &PgPool, action: &str) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM chat.admin_audit WHERE action = $1")
        .bind(format!("public.{action}"))
        .fetch_one(pool)
        .await
        .expect("a count")
}

pub async fn age_membership(pool: &PgPool, user: &str, minutes: i32) {
    sqlx::query(
        "UPDATE chat.members SET joined_at = now() - $3::int * interval '1 minute'
         WHERE conversation_id = $1 AND user_id = $2",
    )
    .bind(ID)
    .bind(user)
    .bind(minutes)
    .execute(pool)
    .await
    .expect("an aged membership");
}

pub async fn slow_mode(pool: &PgPool, seconds: i32) {
    sqlx::query(
        "UPDATE chat.conversations SET settings = jsonb_set(settings, '{slow_mode_s}', to_jsonb($2::int))
         WHERE id = $1",
    )
    .bind(ID)
    .bind(seconds)
    .execute(pool)
    .await
    .expect("a slow mode");
}

pub async fn ask_as(
    chat: &Arc<ChatState>,
    caller: crate::modules::chat::ChatCaller,
    method: &str,
    uri: &str,
    body: Option<Value>,
) -> (u16, Value) {
    let (status, body) = device_call(chat, caller, method, uri, body).await;
    (status.as_u16(), body)
}

pub fn without_cap(user: &str) -> crate::modules::chat::ChatCaller {
    let mut caller = crate::modules::chat::media::support::who(user);
    if let Some(device) = caller.device.as_mut() {
        device.public_chat = false;
    }
    caller
}

pub fn with_cap(user: &str) -> crate::modules::chat::ChatCaller {
    crate::modules::chat::media::support::who(user)
}

pub fn pinging(users: &[&str]) -> Value {
    json!({
        "client_id": Uuid::now_v7(),
        "class": "post",
        "epoch": 0,
        "body": "AQID",
        "pings": users
    })
}

pub async fn ping_seq(pool: &PgPool, user: &str) -> i64 {
    sqlx::query_scalar(
        "SELECT last_ping_seq FROM chat.members WHERE conversation_id = $1 AND user_id = $2",
    )
    .bind(ID)
    .bind(user)
    .fetch_one(pool)
    .await
    .expect("a ping seq")
}

pub async fn read_seq(pool: &PgPool, user: &str) -> i64 {
    sqlx::query_scalar(
        "SELECT last_read_seq FROM chat.members WHERE conversation_id = $1 AND user_id = $2",
    )
    .bind(ID)
    .bind(user)
    .fetch_one(pool)
    .await
    .expect("a read seq")
}
