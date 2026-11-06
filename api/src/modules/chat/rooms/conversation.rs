use chrono::{DateTime, Utc};
use serde_json::Value;
use uuid::Uuid;

use crate::modules::chat::conversations::view::{ConversationView, MembershipView};
use crate::modules::chat::members::access::GroupSettings;
use crate::modules::chat::rooms::lookup::{RoomChatMember, room_of_chat};
use crate::modules::chat::{ChatError, ChatResult, ChatState, history_gate};

struct RoomRow {
    id: Uuid,
    kind: String,
    room_ref: Option<String>,
    created_by: String,
    created_at: DateTime<Utc>,
    last_seq: i64,
    last_at: DateTime<Utc>,
    key_epoch: i32,
    meta_ct: Option<Vec<u8>>,
    meta_epoch: Option<i32>,
    settings: Value,
    member_count: i32,
    ttl_s: Option<i32>,
}

fn role(member: &RoomChatMember) -> &'static str {
    if member.host {
        "owner"
    } else if member.moderator {
        "admin"
    } else {
        "member"
    }
}

pub async fn view(
    chat: &ChatState,
    conversation: Uuid,
    user_id: &str,
) -> ChatResult<ConversationView> {
    let room = room_of_chat(chat, conversation)
        .await?
        .ok_or(ChatError::NotMember)?;
    let now = Utc::now();
    let member =
        RoomChatMember::of(&room, user_id, now.timestamp_millis()).ok_or(ChatError::NotMember)?;
    let row = sqlx::query_file_as!(RoomRow, "queries/chat/rooms/conversation.sql", conversation)
        .fetch_optional(&chat.pg)
        .await?
        .ok_or(ChatError::NotMember)?;
    let joined_epoch =
        history_gate::room_first_epoch(chat, conversation, member.chat_from_seq).await?;
    Ok(ConversationView {
        id: row.id,
        kind: row.kind,
        peer: None,
        created_by: row.created_by,
        created_at: row.created_at.timestamp_millis(),
        last_seq: row.last_seq,
        last_at: row.last_at.timestamp_millis(),
        key_epoch: row.key_epoch,
        meta_ct: row.meta_ct,
        meta_epoch: row.meta_epoch,
        settings: GroupSettings::of(&row.settings),
        member_count: i32::try_from(room.members.len()).unwrap_or(row.member_count),
        ttl_s: row.ttl_s,
        room_ref: row.room_ref,
        pinged: None,
        me: MembershipView {
            role: role(&member).to_owned(),
            state: "active".to_owned(),
            accepted: true,
            joined_seq: member.chat_from_seq,
            joined_epoch,
            last_read_seq: 0,
            muted_until: None,
            notify: "all".to_owned(),
            pinned_at: None,
            archived: false,
            hidden_before_seq: 0,
            invited_by: None,
        },
    })
}
