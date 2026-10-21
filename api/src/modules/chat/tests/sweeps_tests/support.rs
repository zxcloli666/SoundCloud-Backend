use std::sync::Arc;

use chrono::Utc;
use deadpool_redis::Pool;
use redis::AsyncCommands;
use sqlx::PgPool;
use uuid::Uuid;

use crate::modules::chat::rt_keys;
use crate::modules::chat::test_support::chat_state;
use crate::modules::chat::{ChatResult, ChatState};
use crate::modules::rooms::model::{Member, Profile, Room, new_code};
use crate::modules::rooms::store::RoomStore;

pub fn ok<T>(result: ChatResult<T>) -> anyhow::Result<T> {
    result.map_err(|error| anyhow::anyhow!("{error:?}"))
}

pub fn redis_url() -> String {
    std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".to_owned())
}

pub fn redis_db(db: u8) -> String {
    let url = redis_url();
    let base = match url.rsplit_once('/') {
        Some((head, tail)) if !head.ends_with('/') && tail.parse::<u8>().is_ok() => head.to_owned(),
        _ => url.trim_end_matches('/').to_owned(),
    };
    format!("{base}/{db}")
}

pub fn redis() -> Pool {
    crate::redis::connect_url(&redis_url()).expect("a redis pool")
}

pub fn state(pg: PgPool) -> Arc<ChatState> {
    chat_state(pg, redis(), true)
}

pub fn state_on(pg: PgPool, db: u8) -> Arc<ChatState> {
    let redis = crate::redis::connect_url(&redis_db(db)).expect("a redis pool");
    chat_state(pg, redis, true)
}

pub fn user() -> String {
    format!("u{}", Uuid::now_v7().simple())
}

pub fn now_ms() -> i64 {
    Utc::now().timestamp_millis()
}

pub async fn conversation(pg: &PgPool, kind: &str, creator: &str) -> anyhow::Result<Uuid> {
    let id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO chat.conversations (id, kind, created_by, room_alive_until)
         VALUES ($1, $2, $3, CASE WHEN $2 = 'room' THEN now() + interval '10 minutes' END)",
    )
    .bind(id)
    .bind(kind)
    .bind(creator)
    .execute(pg)
    .await?;
    Ok(id)
}

pub async fn member(
    pg: &PgPool,
    conversation: Uuid,
    user: &str,
    state: &str,
) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO chat.members (conversation_id, user_id, state, accepted)
         VALUES ($1, $2, $3, true)",
    )
    .bind(conversation)
    .bind(user)
    .bind(state)
    .execute(pg)
    .await?;
    Ok(())
}

pub async fn messages(
    pg: &PgPool,
    conversation: Uuid,
    seqs: std::ops::RangeInclusive<i64>,
    expires: Option<&str>,
) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO chat.messages
            (conversation_id, seq, client_id, sender_user, class, epoch, body, expires_at)
         SELECT $1, seq, gen_random_uuid(), 'sender', 'post', 0, '\\x00'::bytea,
                now() + $4::text::interval
         FROM generate_series($2::int8, $3::int8) AS seq",
    )
    .bind(conversation)
    .bind(seqs.start())
    .bind(seqs.end())
    .bind(expires)
    .execute(pg)
    .await?;
    sqlx::query("UPDATE chat.conversations SET last_seq = greatest(last_seq, $2) WHERE id = $1")
        .bind(conversation)
        .bind(seqs.end())
        .execute(pg)
        .await?;
    Ok(())
}

pub async fn key(pg: &PgPool, conversation: Uuid, epoch: i32) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO chat.conversation_keys (conversation_id, epoch, wrapped, kek_version)
         VALUES ($1, $2, '\\x01'::bytea, 1)",
    )
    .bind(conversation)
    .bind(epoch)
    .execute(pg)
    .await?;
    Ok(())
}

pub async fn count(pg: &PgPool, sql: &str, conversation: Uuid) -> anyhow::Result<i64> {
    Ok(sqlx::query_scalar(sql)
        .bind(conversation)
        .fetch_one(pg)
        .await?)
}

pub async fn deleted_at_set(pg: &PgPool, conversation: Uuid) -> anyhow::Result<bool> {
    Ok(
        sqlx::query_scalar("SELECT deleted_at IS NOT NULL FROM chat.conversations WHERE id = $1")
            .bind(conversation)
            .fetch_one(pg)
            .await?,
    )
}

pub async fn deltas(chat: &ChatState, user: &str) -> anyhow::Result<Vec<String>> {
    let mut conn = chat.redis.get().await?;
    Ok(conn.zrange(rt_keys::deltas(user), 0, -1).await?)
}

pub async fn room(chat: &ChatState, host: &str, conversation: Uuid) -> anyhow::Result<Room> {
    let code = new_code();
    let profile = Profile {
        name: host.to_owned(),
        avatar_url: None,
    };
    let room = Room::new(
        code.clone(),
        Member::new(host, &profile, now_ms()),
        false,
        now_ms(),
    );
    RoomStore::new(chat.redis.clone())
        .insert(&room)
        .await
        .map_err(|error| anyhow::anyhow!("{error:?}"))?;
    sqlx::query("UPDATE chat.conversations SET room_ref = $2 WHERE id = $1")
        .bind(conversation)
        .bind(&code)
        .execute(&chat.pg)
        .await?;
    Ok(room)
}

pub async fn seen(chat: &ChatState, code: &str, user: &str, at_ms: i64) -> anyhow::Result<()> {
    RoomStore::new(chat.redis.clone())
        .touch(code, user, at_ms)
        .await
        .map_err(|error| anyhow::anyhow!("{error:?}"))
}

pub async fn alive(chat: &ChatState, code: &str, at_ms: i64) -> anyhow::Result<()> {
    let mut conn = chat.redis.get().await?;
    let _: i64 = conn.zadd(rt_keys::ROOMS_ALIVE, code, at_ms).await?;
    Ok(())
}

pub async fn alive_score(chat: &ChatState, code: &str) -> anyhow::Result<Option<f64>> {
    let mut conn = chat.redis.get().await?;
    Ok(conn.zscore(rt_keys::ROOMS_ALIVE, code).await?)
}

pub fn minutes_ago(minutes: i64) -> i64 {
    now_ms() - minutes * 60_000
}
