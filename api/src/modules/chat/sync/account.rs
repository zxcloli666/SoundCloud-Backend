use uuid::Uuid;

use crate::modules::chat::model::{DeviceStatus, DeviceView};
use crate::modules::chat::sync::changes;
use crate::modules::chat::{ChatResult, ChatState};

pub async fn devices(
    chat: &ChatState,
    user_id: &str,
    current: Uuid,
) -> ChatResult<Vec<DeviceView>> {
    let rows = sqlx::query_file!("queries/chat/sync/devices.sql", user_id)
        .fetch_all(&chat.pg)
        .await?;
    Ok(rows
        .into_iter()
        .map(|row| DeviceView {
            id: row.id,
            platform: row.platform,
            name: row.name,
            app_version: row.app_version,
            tier: row.tier,
            status: DeviceStatus::Active,
            created_at: row.created_at.timestamp_millis(),
            last_seen_at: row.last_seen_at.timestamp_millis(),
            history_unlocked: row.history_unlocked,
            current: row.id == current,
        })
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
