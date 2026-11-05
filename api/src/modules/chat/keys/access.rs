use uuid::Uuid;

use crate::modules::chat::model::ConversationKind;
use crate::modules::chat::rooms::lookup::{RoomChatMember, room_of_chat};
use crate::modules::chat::{ChatError, ChatResult, ChatState, history_gate};

pub struct Access {
    pub needs_key: bool,
    pub member_floor: i32,
    pub meta_epoch: Option<i32>,
}

struct Row {
    kind: String,
    rekey: bool,
    deleted: bool,
    member_state: Option<String>,
    joined_epoch: Option<i32>,
    history_visible: bool,
    meta_epoch: Option<i32>,
    has_meta: bool,
    current_key: bool,
}

pub async fn access(chat: &ChatState, conversation: Uuid, user_id: &str) -> ChatResult<Access> {
    let row = sqlx::query_file_as!(Row, "queries/chat/keys/access.sql", conversation, user_id)
        .fetch_optional(&chat.pg)
        .await?
        .filter(|row| !row.deleted)
        .ok_or(ChatError::NotFound)?;
    let (member_floor, shows_image) = if row.kind == ConversationKind::Room.as_str() {
        room_floor(chat, conversation, user_id).await?
    } else {
        (member_floor(&row)?, false)
    };
    let meta_epoch = row.meta_epoch.filter(|_| shows_image && row.has_meta);
    Ok(Access {
        needs_key: row.rekey || !row.current_key,
        member_floor,
        meta_epoch,
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

async fn room_floor(
    chat: &ChatState,
    conversation: Uuid,
    user_id: &str,
) -> ChatResult<(i32, bool)> {
    let room = room_of_chat(chat, conversation)
        .await?
        .ok_or(ChatError::NotMember)?;
    let now = chrono::Utc::now().timestamp_millis();
    let member = RoomChatMember::of(&room, user_id, now).ok_or(ChatError::NotMember)?;
    let floor = history_gate::room_first_epoch(chat, conversation, member.chat_from_seq).await?;
    Ok((floor, room.image_shown().is_some()))
}
