use super::RoomsService;
use crate::error::AppResult;
use crate::modules::chat::rooms::shred;

impl RoomsService {
    pub async fn end(&self, code: &str) -> AppResult<bool> {
        let Some(room) = self.store.load(code).await? else {
            return Ok(false);
        };
        self.store.forget_room(&room).await?;
        self.directory.close(&room).await?;
        self.alive.invalidate(&room.code);
        self.previews.invalidate(&room.code);
        if room.public {
            *self.listing.lock().await = None;
        }
        self.hub.announce(code).await;
        if let Some(chat) = self.chat() {
            shred::end_chat(&chat, &room).await;
        }
        Ok(true)
    }
}
