use chrono::{DateTime, Duration, Utc};
use uuid::Uuid;

use crate::modules::chat::caller::CallerDevice;
use crate::modules::chat::{ChatError, ChatResult, ChatState};

pub const UNLOCK_AFTER_HOURS: i64 = 72;
pub const ALERT_HOURS: [i64; 3] = [0, 24, 48];
pub const NOTHING_VISIBLE: i32 = i32::MAX;

pub fn seasoned(created_at: DateTime<Utc>, now: DateTime<Utc>) -> bool {
    now - created_at >= Duration::hours(UNLOCK_AFTER_HOURS)
}

pub fn is_unlocked(created_at: DateTime<Utc>, confirmed: bool, now: DateTime<Utc>) -> bool {
    confirmed || seasoned(created_at, now)
}

pub async fn first_visible_epoch(
    chat: &ChatState,
    device: &CallerDevice,
    conversation: Uuid,
) -> ChatResult<i32> {
    if is_unlocked(device.created_at, device.history_unlocked, Utc::now()) {
        return Ok(0);
    }
    let first = sqlx::query_file_scalar!(
        "queries/chat/keys/first_epoch_after.sql",
        conversation,
        device.created_at
    )
    .fetch_optional(&chat.pg)
    .await?;
    Ok(first.unwrap_or(NOTHING_VISIBLE))
}

pub async fn room_first_epoch(
    chat: &ChatState,
    conversation: Uuid,
    from_seq: i64,
) -> ChatResult<i32> {
    sqlx::query_file_scalar!(
        "queries/chat/keys/room_first_epoch.sql",
        conversation,
        from_seq
    )
    .fetch_optional(&chat.pg)
    .await?
    .ok_or(ChatError::NotFound)
}
