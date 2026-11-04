use uuid::Uuid;

use crate::modules::chat::rooms::refusal;
use crate::modules::chat::{ChatResult, ChatState, sync};
use crate::modules::rooms::store::RoomStore;

pub async fn devices_revoked(chat: &ChatState, user_id: &str, ids: &[Uuid]) -> ChatResult<()> {
    chat.devices.forget(user_id, ids);
    sync::bump::devices(chat, user_id).await
}

pub async fn room_seen_since(chat: &ChatState, code: &str, since_ms: i64) -> ChatResult<bool> {
    let seen = RoomStore::new(chat.redis.clone())
        .seen_since(code, since_ms)
        .await
        .map_err(refusal)?;
    Ok(!seen.is_empty())
}

pub async fn end_room(chat: &ChatState, code: &str) -> ChatResult<bool> {
    chat.rooms.end(code).await.map_err(refusal)
}
