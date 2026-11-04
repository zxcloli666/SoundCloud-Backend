use super::RoomsService;
use crate::error::AppResult;
use crate::modules::chat::rooms::bridge;
use crate::modules::rooms::model::Room;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cause<'a> {
    Member,
    Moderation { by: &'a str, ban: bool },
    Quiet,
}

impl RoomsService {
    pub async fn apply<T, F>(
        &self,
        code: &str,
        cause: Cause<'_>,
        mut change: F,
    ) -> AppResult<(Room, T)>
    where
        F: FnMut(&mut Room) -> AppResult<T>,
    {
        let mut before = None;
        let (after, outcome) = self
            .store
            .update(code, |room| {
                before = Some(room.clone());
                change(room)
            })
            .await?;
        if let Some(before) = before {
            self.settle(&before, &after, cause).await;
        }
        Ok((after, outcome))
    }

    pub async fn relocate<F>(&self, code: &str, cause: Cause<'_>, mut change: F) -> AppResult<Room>
    where
        F: FnMut(&mut Room) -> AppResult<()>,
    {
        let mut before = None;
        let after = self
            .store
            .relocate(code, |room| {
                before = Some(room.clone());
                change(room)
            })
            .await?;
        self.store.forget_alive(code).await?;
        self.alive.invalidate(&code.to_owned());
        self.previews.invalidate(&code.to_owned());
        if let Some(chat_id) = after.chat_id {
            self.store.index_chat(chat_id, &after.code).await?;
        }
        if let Some(before) = before {
            self.settle(&before, &after, cause).await;
        }
        Ok(after)
    }

    async fn settle(&self, before: &Room, after: &Room, cause: Cause<'_>) {
        self.hub.announce(&before.code).await;
        if let Some(chat) = self.chat() {
            bridge::after_change(&chat, before, after, cause).await;
        }
    }
}
