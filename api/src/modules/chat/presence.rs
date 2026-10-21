use std::time::Duration;

use chrono::Utc;

use crate::modules::chat::sync::prefs;
use crate::modules::chat::{ChatResult, ChatState, rt_keys};

pub const ONLINE_WINDOW: Duration = Duration::from_secs(180);

pub async fn touch(chat: &ChatState, user_id: &str) {
    if !chat.hub.seen_due(user_id) {
        return;
    }
    if let Err(error) = mark_seen(chat, user_id).await {
        tracing::debug!(?error, "presence was not marked");
    }
}

pub async fn mark_seen(chat: &ChatState, user_id: &str) -> ChatResult<()> {
    if !prefs::of(chat, user_id).await?.presence_share {
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

pub async fn online_contacts(chat: &ChatState, user_id: &str) -> ChatResult<Vec<String>> {
    let contacts = sqlx::query_file_scalar!("queries/chat/sync/contacts.sql", user_id)
        .fetch_all(&chat.pg)
        .await?;
    if contacts.is_empty() {
        return Ok(Vec::new());
    }
    let seen: Vec<Option<f64>> = {
        let mut conn = chat.redis.get().await?;
        redis::cmd("ZMSCORE")
            .arg(rt_keys::SEEN)
            .arg(&contacts)
            .query_async(&mut conn)
            .await?
    };
    let horizon = (Utc::now().timestamp_millis() - millis(ONLINE_WINDOW)) as f64;
    let mut online: Vec<String> = contacts
        .into_iter()
        .zip(seen)
        .filter_map(|(contact, at)| at.filter(|at| *at >= horizon).map(|_| contact))
        .collect();
    online.sort();
    Ok(online)
}

fn millis(duration: Duration) -> i64 {
    i64::try_from(duration.as_millis()).unwrap_or(i64::MAX)
}
