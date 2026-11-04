use std::time::Duration;

use chrono::Utc;
use uuid::Uuid;

use crate::modules::chat::rooms::lookup::room_of_chat;
use crate::modules::chat::sweeps::{hooks, purge};
use crate::modules::chat::{ChatResult, ChatState, rt_keys};

pub const EVERY_SECS: u64 = 30;
pub const DEAD_AFTER: Duration = Duration::from_secs(10 * 60);
pub const ROOMS_PER_RUN: isize = 200;
pub const CHATS_PER_RUN: i64 = 200;

pub async fn run_once(chat: &ChatState) -> ChatResult<u64> {
    let since = Utc::now().timestamp_millis() - DEAD_AFTER.as_millis() as i64;
    let mut reaped = 0;
    for code in quiet_rooms(chat, since).await? {
        if reap_room(chat, &code, since).await? {
            reaped += 1;
        }
    }
    let lapsed =
        sqlx::query_file_scalar!("queries/chat/sweeps/lapsed_room_chats.sql", CHATS_PER_RUN)
            .fetch_all(&chat.pg)
            .await?;
    for conversation in lapsed {
        if reap_chat(chat, conversation, since).await? {
            reaped += 1;
        }
    }
    metrics::counter!("chat_rooms_reaped_total").increment(reaped);
    Ok(reaped)
}

async fn quiet_rooms(chat: &ChatState, since: i64) -> ChatResult<Vec<String>> {
    let mut conn = chat.redis.get().await?;
    let codes: Vec<String> = redis::cmd("ZRANGEBYSCORE")
        .arg(rt_keys::ROOMS_ALIVE)
        .arg("-inf")
        .arg(format!("({since}"))
        .arg("LIMIT")
        .arg(0)
        .arg(ROOMS_PER_RUN)
        .query_async(&mut conn)
        .await?;
    Ok(codes)
}

async fn reap_room(chat: &ChatState, code: &str, since: i64) -> ChatResult<bool> {
    if hooks::room_seen_since(chat, code, since).await? {
        return Ok(false);
    }
    if hooks::end_room(chat, code).await? {
        return Ok(true);
    }
    let mut conn = chat.redis.get().await?;
    let _: i64 = redis::cmd("ZREM")
        .arg(rt_keys::ROOMS_ALIVE)
        .arg(code)
        .query_async(&mut conn)
        .await?;
    Ok(false)
}

async fn reap_chat(chat: &ChatState, conversation: Uuid, since: i64) -> ChatResult<bool> {
    match room_of_chat(chat, conversation).await? {
        Some(room) if hooks::room_seen_since(chat, &room.code, since).await? => Ok(false),
        Some(room) => {
            hooks::end_room(chat, &room.code).await?;
            purge::shred(chat, conversation).await?;
            Ok(true)
        }
        None => purge::shred(chat, conversation).await,
    }
}
