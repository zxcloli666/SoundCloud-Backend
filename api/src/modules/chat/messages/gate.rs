use sc_chat_wire::msg::Class;
use sqlx::PgPool;

use crate::modules::chat::limits::{self, RoomPost};
use crate::modules::chat::members::access::Access;
use crate::modules::chat::messages::rules::Outgoing;
use crate::modules::chat::messages::store;
use crate::modules::chat::model::{ConversationKind, MemberState, MessageRow};
use crate::modules::chat::requests::MESSAGES_BEFORE_ACCEPT;
use crate::modules::chat::rooms::lookup;
use crate::modules::chat::users::{self, Scope};
use crate::modules::chat::{ChatError, ChatResult, ChatState};

pub fn peer_open(access: &Access) -> ChatResult<()> {
    if access.kind != ConversationKind::Direct {
        return Ok(());
    }
    match access.peer_state {
        Some(MemberState::Active | MemberState::Invited) => Ok(()),
        _ => Err(ChatError::Unavailable),
    }
}

pub async fn request_room(pg: &PgPool, access: &Access, out: &Outgoing) -> ChatResult<()> {
    if out.class != Class::Post || access.peer_state != Some(MemberState::Invited) {
        return Ok(());
    }
    let sent = sqlx::query_file_scalar!(
        "queries/chat/messages/posts_by.sql",
        access.conversation,
        access.user_id,
        out.client_id
    )
    .fetch_one(pg)
    .await?;
    if sent >= MESSAGES_BEFORE_ACCEPT {
        return Err(ChatError::RequestLimit);
    }
    Ok(())
}

pub async fn sender_free(chat: &ChatState, access: &Access) -> ChatResult<()> {
    let scope = if access.peer_state == Some(MemberState::Invited) {
        Scope::DmNew
    } else {
        Scope::All
    };
    users::check(chat, &access.user_id, scope).await
}

pub async fn room(
    chat: &ChatState,
    access: &Access,
    out: &Outgoing,
) -> ChatResult<Option<MessageRow>> {
    let Err(refused) = room_open(chat, access, out).await else {
        return Ok(None);
    };
    store::existing(
        &chat.pg,
        access.conversation,
        out.client_id,
        &access.user_id,
    )
    .await?
    .map(Some)
    .ok_or(refused)
}

async fn room_open(chat: &ChatState, access: &Access, out: &Outgoing) -> ChatResult<()> {
    let seat = lookup::gate_send(
        chat,
        access.conversation,
        &access.user_id,
        out.class.as_str(),
    )
    .await?;
    let seated_ms = chrono::Utc::now().timestamp_millis() - seat.joined_at;
    let post = RoomPost {
        conversation: access.conversation,
        public: access.public_room,
        member_for_secs: seated_ms / 1_000,
        body_len: out.body.len(),
    };
    limits::admit_room_post(chat, &access.user_id, &post).await
}
