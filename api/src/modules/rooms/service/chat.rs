use uuid::Uuid;

use super::{Cause, RoomsService};
use crate::error::{AppError, AppResult};
use crate::modules::chat::rooms::bridge;
use crate::modules::rooms::model::{Room, not_member};

#[derive(Debug, Clone)]
pub struct OpenedChat {
    pub room: Room,
    pub conversation_id: Uuid,
    pub from_seq: i64,
}

fn chat_off() -> AppError {
    AppError::coded(
        axum::http::StatusCode::FORBIDDEN,
        "room_chat_disabled",
        "The host turned the room chat off",
    )
}

impl RoomsService {
    pub async fn claim_chat(&self, code: &str, user_id: &str) -> AppResult<OpenedChat> {
        let room = self.room(code).await?;
        let from_seq = room.member(user_id).ok_or_else(not_member)?.chat_from_seq;
        if !room.chat_enabled {
            return Err(chat_off());
        }
        let (room, conversation_id) = match room.chat_id {
            Some(id) => (room, id),
            None => {
                self.apply(code, Cause::Quiet, |room| {
                    if !room.is_member(user_id) {
                        return Err(not_member());
                    }
                    Ok(*room.chat_id.get_or_insert_with(Uuid::now_v7))
                })
                .await?
            }
        };
        self.store.index_chat(conversation_id, &room.code).await?;
        Ok(OpenedChat {
            room,
            conversation_id,
            from_seq,
        })
    }

    pub async fn room_by_chat(&self, conversation: Uuid) -> AppResult<Option<Room>> {
        let Some(code) = self.store.chat_code(conversation).await? else {
            return Ok(None);
        };
        Ok(self
            .store
            .load(&code)
            .await?
            .filter(|room| room.chat_id == Some(conversation)))
    }

    pub(super) async fn chat_last_seq(&self, room: &Room) -> i64 {
        match (self.chat(), room.chat_id) {
            (Some(chat), Some(conversation)) => bridge::last_seq(&chat, conversation).await,
            _ => 0,
        }
    }
}
