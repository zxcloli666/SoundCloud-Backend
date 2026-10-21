use std::collections::HashMap;

use uuid::Uuid;

use crate::modules::chat::model::ConversationKind;
use crate::modules::chat::{ChatResult, ChatState, rooms};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Seat {
    Live { floor: i64 },
    Gone,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConversationState {
    pub id: Uuid,
    pub kind: String,
    pub last_seq: i64,
    pub key_epoch: i32,
    pub seat: Seat,
}

impl ConversationState {
    pub fn is_direct(&self) -> bool {
        self.kind == ConversationKind::Direct.as_str()
    }
}

struct Row {
    id: Uuid,
    kind: String,
    last_seq: i64,
    key_epoch: i32,
    deleted: bool,
    state: Option<String>,
    joined_seq: Option<i64>,
    hidden_before_seq: Option<i64>,
}

impl Row {
    fn is_room(&self) -> bool {
        self.kind == ConversationKind::Room.as_str()
    }

    fn member_seat(&self) -> Seat {
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
}

pub fn seat_of(states: &HashMap<Uuid, ConversationState>, id: Uuid) -> Seat {
    states.get(&id).map_or(Seat::Gone, |state| state.seat)
}

pub async fn load(
    chat: &ChatState,
    user_id: &str,
    ids: &[Uuid],
) -> ChatResult<HashMap<Uuid, ConversationState>> {
    if ids.is_empty() {
        return Ok(HashMap::new());
    }
    let rows = sqlx::query_file_as!(Row, "queries/chat/sync/conversations.sql", ids, user_id)
        .fetch_all(&chat.pg)
        .await?;
    let mut states = HashMap::with_capacity(rows.len());
    for row in rows {
        let seat = seat(chat, user_id, &row).await?;
        states.insert(
            row.id,
            ConversationState {
                id: row.id,
                kind: row.kind,
                last_seq: row.last_seq,
                key_epoch: row.key_epoch,
                seat,
            },
        );
    }
    Ok(states)
}

async fn seat(chat: &ChatState, user_id: &str, row: &Row) -> ChatResult<Seat> {
    if row.deleted {
        return Ok(Seat::Gone);
    }
    if !row.is_room() {
        return Ok(row.member_seat());
    }
    let from_seq = rooms::member_from_seq(chat, row.id, user_id).await?;
    Ok(from_seq.map_or(Seat::Gone, |floor| Seat::Live { floor }))
}
