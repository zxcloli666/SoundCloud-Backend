use chrono::Utc;
use uuid::Uuid;

use crate::modules::chat::model::DeviceView;
use crate::modules::chat::sync::changes;
use crate::modules::chat::{ChatResult, ChatState, devices};

pub async fn devices(
    chat: &ChatState,
    user_id: &str,
    current: Uuid,
) -> ChatResult<Vec<DeviceView>> {
    let now = Utc::now();
    let rows = devices::live_of(chat, user_id).await?;
    Ok(rows
        .iter()
        .map(|row| devices::view(row, Some(current), now))
        .collect())
}

pub async fn requests_count(chat: &ChatState, user_id: &str) -> ChatResult<u32> {
    let count = sqlx::query_file_scalar!("queries/chat/sync/requests_count.sql", user_id)
        .fetch_one(&chat.pg)
        .await?;
    let count = u32::try_from(count).unwrap_or(u32::MAX);
    changes::remember_requests(chat, user_id, count).await?;
    Ok(count)
}
