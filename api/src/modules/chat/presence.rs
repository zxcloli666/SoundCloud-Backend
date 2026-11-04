use std::time::Duration;

use chrono::Utc;
use redis::AsyncCommands;
use serde::Serialize;

use crate::modules::chat::sync::prefs;
use crate::modules::chat::{ChatResult, ChatState, rt_keys};

pub const ONLINE_WINDOW: Duration = Duration::from_secs(180);

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct PresenceView {
    pub online: bool,
    pub last_seen_at: Option<i64>,
}

pub async fn touch(chat: &ChatState, user_id: &str) {
    if !chat.hub.seen_due(user_id) {
        return;
    }
    if let Err(error) = mark_seen(chat, user_id).await {
        tracing::debug!(?error, "presence was not marked");
    }
}

pub async fn mark_seen(chat: &ChatState, user_id: &str) -> ChatResult<()> {
    if !prefs::shares_presence(chat, user_id).await? {
        return Ok(());
    }
    let now = Utc::now().timestamp_millis();
    let horizon = now - millis(rt_keys::SEEN_TTL);
    let mut conn = chat.redis.get().await?;
    let _: () = redis::pipe()
        .zadd(rt_keys::SEEN, user_id, now)
        .ignore()
        .zrembyscore(rt_keys::SEEN, "-inf", horizon)
        .ignore()
        .expire(rt_keys::SEEN, rt_keys::SEEN_TTL.as_secs() as i64)
        .ignore()
        .query_async(&mut conn)
        .await?;
    Ok(())
}

pub async fn forget(chat: &ChatState, user_id: &str) -> ChatResult<()> {
    let mut conn = chat.redis.get().await?;
    let _: i64 = conn.zrem(rt_keys::SEEN, user_id).await?;
    Ok(())
}

pub async fn of_contact(
    chat: &ChatState,
    viewer: &str,
    user_id: &str,
) -> ChatResult<Option<PresenceView>> {
    if viewer == user_id {
        return Ok(None);
    }
    let visible = sqlx::query_file_scalar!("queries/chat/presence/visible.sql", viewer, user_id)
        .fetch_one(&chat.pg)
        .await?;
    if !visible {
        return Ok(None);
    }
    let seen = match last_seen(chat, user_id).await {
        Ok(seen) => seen,
        Err(error) => {
            tracing::debug!(code = error.code(), "presence was not read");
            return Ok(None);
        }
    };
    let horizon = Utc::now().timestamp_millis() - millis(ONLINE_WINDOW);
    Ok(Some(PresenceView {
        online: seen.is_some_and(|at| at >= horizon),
        last_seen_at: seen,
    }))
}

async fn last_seen(chat: &ChatState, user_id: &str) -> ChatResult<Option<i64>> {
    let mut conn = chat.redis.get().await?;
    let at: Option<f64> = conn.zscore(rt_keys::SEEN, user_id).await?;
    Ok(at.map(|at| at as i64))
}

fn millis(duration: Duration) -> i64 {
    i64::try_from(duration.as_millis()).unwrap_or(i64::MAX)
}
