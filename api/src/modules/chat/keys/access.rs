use uuid::Uuid;

use crate::modules::chat::model::ConversationKind;
use crate::modules::chat::{ChatError, ChatResult, ChatState, history_gate, rooms};

pub struct Access {
    pub needs_key: bool,
    pub member_floor: i32,
}

struct Row {
    kind: String,
    rekey: bool,
    deleted: bool,
    member_state: Option<String>,
    joined_epoch: Option<i32>,
    history_visible: bool,
    current_key: bool,
}

pub async fn access(chat: &ChatState, conversation: Uuid, user_id: &str) -> ChatResult<Access> {
    let row = sqlx::query_file_as!(Row, "queries/chat/keys/access.sql", conversation, user_id)
        .fetch_optional(&chat.pg)
        .await?
        .filter(|row| !row.deleted)
        .ok_or(ChatError::NotFound)?;
    let member_floor = if row.kind == ConversationKind::Room.as_str() {
        room_floor(chat, conversation, user_id).await?
    } else {
        member_floor(&row)?
    };
    Ok(Access {
        needs_key: row.rekey || !row.current_key,
        member_floor,
    })
}

fn member_floor(row: &Row) -> ChatResult<i32> {
    match row.member_state.as_deref() {
        Some("active" | "invited") => {}
        _ => return Err(ChatError::NotMember),
    }
    let hides_history = row.kind == ConversationKind::Group.as_str() && !row.history_visible;
    Ok(if hides_history {
        row.joined_epoch.unwrap_or(0)
    } else {
        0
    })
}

async fn room_floor(chat: &ChatState, conversation: Uuid, user_id: &str) -> ChatResult<i32> {
    let from_seq = rooms::member_from_seq(chat, conversation, user_id)
        .await?
        .ok_or(ChatError::NotMember)?;
    history_gate::room_first_epoch(chat, conversation, from_seq).await
}
