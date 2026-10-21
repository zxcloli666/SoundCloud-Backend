use std::collections::HashMap;

use uuid::Uuid;

use crate::modules::chat::model::ConversationKind;
use crate::modules::chat::{ChatResult, ChatState};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Seat {
    Live { floor: i64 },
    Gone,
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConversationState {
    pub id: Uuid,
    pub kind: String,
    pub last_seq: i64,
    pub key_epoch: i32,
    pub deleted: bool,
    pub state: Option<String>,
    pub joined_seq: Option<i64>,
    pub hidden_before_seq: Option<i64>,
}

impl ConversationState {
    pub fn seat(&self) -> Seat {
        if self.deleted {
            return Seat::Gone;
        }
        if self.kind == ConversationKind::Room.as_str() {
            return Seat::Unknown;
        }
        match self.state.as_deref() {
            Some("active" | "invited") => Seat::Live {
                floor: self
                    .joined_seq
                    .unwrap_or(0)
                    .max(self.hidden_before_seq.unwrap_or(0)),
            },
            _ => Seat::Gone,
        }
    }

    pub fn is_direct(&self) -> bool {
        self.kind == ConversationKind::Direct.as_str()
    }
}

pub fn seat_of(states: &HashMap<Uuid, ConversationState>, id: Uuid) -> Seat {
    states.get(&id).map_or(Seat::Gone, ConversationState::seat)
}

pub async fn load(
    chat: &ChatState,
    user_id: &str,
    ids: &[Uuid],
) -> ChatResult<HashMap<Uuid, ConversationState>> {
    if ids.is_empty() {
        return Ok(HashMap::new());
    }
    let rows = sqlx::query_file_as!(
        ConversationState,
        "queries/chat/sync/conversations.sql",
        ids,
        user_id
    )
    .fetch_all(&chat.pg)
    .await?;
    Ok(rows.into_iter().map(|row| (row.id, row)).collect())
}
