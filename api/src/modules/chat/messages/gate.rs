use chat_wire::msg::Class;
use sqlx::PgPool;

use crate::modules::chat::members::access::Access;
use crate::modules::chat::model::{ConversationKind, MemberState};
use crate::modules::chat::requests::MESSAGES_BEFORE_ACCEPT;
use crate::modules::chat::{ChatError, ChatResult};

pub fn peer_open(access: &Access) -> ChatResult<()> {
    if access.kind != ConversationKind::Direct {
        return Ok(());
    }
    match access.peer_state {
        Some(MemberState::Active | MemberState::Invited) => Ok(()),
        _ => Err(ChatError::Unavailable),
    }
}

pub async fn request_room(pg: &PgPool, access: &Access, class: Class) -> ChatResult<()> {
    if class != Class::Post || access.peer_state != Some(MemberState::Invited) {
        return Ok(());
    }
    let sent = sqlx::query_file_scalar!(
        "queries/chat/messages/posts_by.sql",
        access.conversation,
        access.user_id
    )
    .fetch_one(pg)
    .await?;
    if sent >= MESSAGES_BEFORE_ACCEPT {
        return Err(ChatError::RequestLimit);
    }
    Ok(())
}
