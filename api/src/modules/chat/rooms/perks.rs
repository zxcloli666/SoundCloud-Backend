use crate::modules::chat::rooms::refusal;
use crate::modules::chat::sweeps::rooms::DEAD_AFTER;
use crate::modules::chat::{ChatResult, ChatState};

pub const EVERY_SECS: u64 = 600;

pub async fn run_once(chat: &ChatState) -> ChatResult<u64> {
    let since = chrono::Utc::now().timestamp_millis() - DEAD_AFTER.as_millis() as i64;
    let codes = chat.rooms.live_since(since).await.map_err(refusal)?;
    let changed = chat.rooms.recheck_perks(&codes).await.map_err(refusal)?;
    metrics::counter!("chat_room_perks_changed_total").increment(changed);
    Ok(changed)
}
