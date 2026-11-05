use redis::AsyncCommands;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::modules::chat::caller::{CallerBind, SessionEntry};
use crate::modules::chat::synthetic::is_synthetic;
use crate::modules::chat::{ChatResult, ChatState, rt_keys};

pub const WRITES_PER_CALL: usize = 1_000;

#[derive(Serialize, Deserialize)]
struct Stored {
    user_id: String,
    #[serde(with = "crate::modules::chat::b64")]
    bind: Vec<u8>,
}

pub struct NewSession<'a> {
    pub id: Uuid,
    pub user_id: &'a str,
    pub bind: Vec<u8>,
}

pub async fn session(chat: &ChatState, id: Uuid) -> ChatResult<Option<SessionEntry>> {
    if !chat.config.loadtest_open() {
        return Ok(None);
    }
    let raw: Option<String> = {
        let mut conn = chat.redis.get().await?;
        conn.get(rt_keys::loadtest_session(id)).await?
    };
    Ok(raw.as_deref().and_then(decode))
}

pub fn encode(user_id: &str, bind: &[u8]) -> String {
    let stored = Stored {
        user_id: user_id.to_owned(),
        bind: bind.to_vec(),
    };
    serde_json::to_string(&stored).unwrap_or_default()
}

pub fn decode(raw: &str) -> Option<SessionEntry> {
    let stored: Stored = serde_json::from_str(raw).ok()?;
    if !is_synthetic(&stored.user_id) {
        return None;
    }
    let bind = CallerBind::decode(&stored.bind)?;
    Some(SessionEntry {
        user_id: stored.user_id,
        bind: Some(bind),
    })
}

pub async fn store(chat: &ChatState, sessions: &[NewSession<'_>]) -> ChatResult<()> {
    let ttl = rt_keys::LOADTEST_SESSION_TTL.as_secs();
    for batch in sessions.chunks(WRITES_PER_CALL) {
        let mut pipe = redis::pipe();
        for session in batch {
            pipe.set_ex(
                rt_keys::loadtest_session(session.id),
                encode(session.user_id, &session.bind),
                ttl,
            )
            .ignore();
        }
        let mut conn = chat.redis.get().await?;
        let _: () = pipe.query_async(&mut conn).await?;
    }
    Ok(())
}

pub async fn drop_all(chat: &ChatState) -> ChatResult<u64> {
    let mut dropped = 0;
    let mut cursor = 0u64;
    loop {
        let mut conn = chat.redis.get().await?;
        let (next, keys): (u64, Vec<String>) = redis::cmd("SCAN")
            .arg(cursor)
            .arg("MATCH")
            .arg(rt_keys::LOADTEST_SESSIONS)
            .arg("COUNT")
            .arg(WRITES_PER_CALL)
            .query_async(&mut conn)
            .await?;
        if !keys.is_empty() {
            let removed: u64 = conn.del(&keys).await?;
            dropped += removed;
        }
        for id in keys
            .iter()
            .filter_map(|key| rt_keys::loadtest_session_id(key))
        {
            chat.callers.forget_session(id);
        }
        if next == 0 {
            return Ok(dropped);
        }
        cursor = next;
    }
}
