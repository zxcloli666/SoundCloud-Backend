use crate::modules::chat::rooms::refusal;
use crate::modules::chat::sweeps::rooms::DEAD_AFTER;
use crate::modules::chat::{ChatResult, ChatState};
use crate::modules::rooms::model::Room;

pub const EVERY_SECS: u64 = 600;

pub async fn run_once(chat: &ChatState) -> ChatResult<u64> {
    let since = chrono::Utc::now().timestamp_millis() - DEAD_AFTER.as_millis() as i64;
    let codes = chat.rooms.live_since(since).await.map_err(refusal)?;
    let changed = chat.rooms.recheck_perks(&codes).await.map_err(refusal)?;
    metrics::counter!("chat_room_perks_changed_total").increment(changed);
    Ok(changed)
}

pub async fn upgraded(chat: &ChatState, room: &Room, user_id: &str) -> ChatResult<Option<Room>> {
    if !room.is_host(user_id) || room.perks.star_host {
        return Ok(None);
    }
    if !chat.stars.is_star_now(user_id).await? {
        return Ok(None);
    }
    let after = chat
        .rooms
        .upgrade_to_star(&room.code, user_id)
        .await
        .map_err(refusal)?;
    if !after.perks.star_host {
        return Ok(None);
    }
    metrics::counter!("chat_room_perks_changed_total").increment(1);
    Ok(Some(after))
}
