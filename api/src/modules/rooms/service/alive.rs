use super::RoomsService;
use crate::error::AppResult;
use crate::modules::chat::rooms::bridge;
use crate::modules::rooms::model::Room;

impl RoomsService {
    pub(super) async fn keep_alive(&self, room: &Room, now: i64) -> AppResult<()> {
        if self.alive.contains_key(&room.code) {
            return Ok(());
        }
        self.alive.insert(room.code.clone(), ());
        self.store.keep_alive(room, now).await?;
        if let (Some(chat), Some(conversation)) = (self.chat(), room.chat_id) {
            bridge::keep_alive(&chat, conversation).await;
        }
        Ok(())
    }
}
