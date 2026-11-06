use std::sync::LazyLock;

use chrono::Utc;
use redis::Script;

use crate::modules::chat::public_chat::{self, ID};
use crate::modules::chat::{ChatError, ChatResult, ChatState, rt_keys};

static REFRESH: LazyLock<Script> =
    LazyLock::new(|| Script::new(include_str!("../lua/public_refresh.lua")));

pub const TTL_MS: i64 = 60_000;
const KEY_TTL_S: i64 = 180;
const GAUGE: &str = "chat_public_watchers";

fn cutoff() -> i64 {
    Utc::now().timestamp_millis() - TTL_MS
}

pub async fn touch(chat: &ChatState, user_id: &str) -> ChatResult<()> {
    if refresh(chat, user_id).await? {
        return Ok(());
    }
    let active = sqlx::query_file_scalar!("queries/chat/public/active.sql", ID, user_id)
        .fetch_one(&chat.pg)
        .await?;
    if !active {
        return Err(ChatError::NotMember);
    }
    public_chat::gate::read(chat, user_id).await?;
    add(chat, user_id).await
}

async fn refresh(chat: &ChatState, user_id: &str) -> ChatResult<bool> {
    let mut conn = chat.redis.get().await?;
    let refreshed: i64 = REFRESH
        .key(rt_keys::PUBLIC_WATCHERS)
        .arg(user_id)
        .arg(Utc::now().timestamp_millis())
        .arg(KEY_TTL_S)
        .invoke_async(&mut conn)
        .await?;
    Ok(refreshed == 1)
}

async fn add(chat: &ChatState, user_id: &str) -> ChatResult<()> {
    let mut conn = chat.redis.get().await?;
    let _: () = redis::pipe()
        .zadd(
            rt_keys::PUBLIC_WATCHERS,
            user_id,
            Utc::now().timestamp_millis(),
        )
        .ignore()
        .expire(rt_keys::PUBLIC_WATCHERS, KEY_TTL_S)
        .ignore()
        .query_async(&mut conn)
        .await?;
    Ok(())
}

pub async fn remove(chat: &ChatState, user_id: &str) -> ChatResult<()> {
    let mut conn = chat.redis.get().await?;
    let _: i64 = redis::cmd("ZREM")
        .arg(rt_keys::PUBLIC_WATCHERS)
        .arg(user_id)
        .query_async(&mut conn)
        .await?;
    Ok(())
}

pub async fn current(chat: &ChatState) -> ChatResult<Vec<String>> {
    let since = cutoff();
    let mut conn = chat.redis.get().await?;
    let (_, users): (i64, Vec<String>) = redis::pipe()
        .cmd("ZREMRANGEBYSCORE")
        .arg(rt_keys::PUBLIC_WATCHERS)
        .arg("-inf")
        .arg(format!("({since}"))
        .cmd("ZRANGEBYSCORE")
        .arg(rt_keys::PUBLIC_WATCHERS)
        .arg(since)
        .arg("+inf")
        .query_async(&mut conn)
        .await?;
    metrics::gauge!(GAUGE).set(users.len() as f64);
    Ok(users)
}

pub async fn count(chat: &ChatState) -> ChatResult<u32> {
    let mut conn = chat.redis.get().await?;
    let online: u32 = redis::cmd("ZCOUNT")
        .arg(rt_keys::PUBLIC_WATCHERS)
        .arg(cutoff())
        .arg("+inf")
        .query_async(&mut conn)
        .await?;
    Ok(online)
}
