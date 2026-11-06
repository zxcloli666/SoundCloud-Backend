use chrono::Utc;
use sc_chat_wire::msg::Class;

use crate::modules::chat::limits::{self, RoomPost};
use crate::modules::chat::members::access::Access;
use crate::modules::chat::messages::rules::Outgoing;
use crate::modules::chat::messages::store;
use crate::modules::chat::model::MessageRow;
use crate::modules::chat::public_chat::ID;
use crate::modules::chat::users::{self, Scope};
use crate::modules::chat::{ChatError, ChatResult, ChatState, slow_mode};

pub async fn read(chat: &ChatState, user_id: &str) -> ChatResult<()> {
    users::check(chat, user_id, Scope::All).await
}

pub async fn send(
    chat: &ChatState,
    access: &Access,
    out: &Outgoing,
) -> ChatResult<Option<MessageRow>> {
    let Err(refused) = open(chat, access, out).await else {
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

async fn open(chat: &ChatState, access: &Access, out: &Outgoing) -> ChatResult<()> {
    let standing = sqlx::query_file!("queries/chat/public/standing.sql", ID, access.user_id)
        .fetch_optional(&chat.pg)
        .await?
        .ok_or(ChatError::NotMember)?;
    if let Some(until) = standing.muted_until {
        return Err(ChatError::Muted {
            until: until.timestamp_millis(),
        });
    }
    let slow_mode_s = access.settings.slow_mode_s;
    if out.class == Class::Post && slow_mode_s > 0 && !access.is_admin() {
        slow_mode::hold(chat, ID, &access.user_id, slow_mode_s).await?;
    }
    let member_for_secs = standing
        .joined_at
        .map_or(i64::MAX, |at| (Utc::now() - at).num_seconds());
    let post = RoomPost {
        conversation: ID,
        public: true,
        member_for_secs,
        body_len: out.body.len(),
    };
    limits::admit_room_post(chat, &access.user_id, &post).await
}
