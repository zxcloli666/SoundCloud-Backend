use redis::AsyncCommands;
use uuid::Uuid;

use crate::modules::chat::rooms::refusal;
use crate::modules::chat::rt_keys;
use crate::modules::chat::{ChatError, ChatResult, ChatState};
use crate::modules::rooms::model::Room;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoomChatMember {
    pub user_id: String,
    pub code: String,
    pub chat_from_seq: i64,
    pub joined_at: i64,
    pub host: bool,
    pub moderator: bool,
    pub muted_until: Option<i64>,
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

impl RoomChatMember {
    pub fn of(room: &Room, user_id: &str, now: i64) -> Option<Self> {
        let member = room.member(user_id)?;
        Some(Self {
            user_id: member.user_id.clone(),
            code: room.code.clone(),
            chat_from_seq: member.chat_from_seq,
            joined_at: member.joined_at,
            host: room.is_host(user_id),
            moderator: room.is_moderator(user_id),
            muted_until: room.muted_until(user_id, now),
        })
    }

    pub fn can_moderate(&self) -> bool {
        self.host || self.moderator
    }
}

pub async fn chat_of_room(chat: &ChatState, code: &str) -> ChatResult<Option<Uuid>> {
    let room = chat.rooms.find(code).await.map_err(refusal)?;
    Ok(room.and_then(|room| room.chat_id))
}

pub async fn room_of_chat(chat: &ChatState, conversation: Uuid) -> ChatResult<Option<Room>> {
    chat.rooms.room_by_chat(conversation).await.map_err(refusal)
}

pub async fn member_of_chat(
    chat: &ChatState,
    conversation: Uuid,
    user_id: &str,
) -> ChatResult<Option<RoomChatMember>> {
    let room = room_of_chat(chat, conversation).await?;
    Ok(room.and_then(|room| RoomChatMember::of(&room, user_id, now_ms())))
}

pub async fn recipients(chat: &ChatState, conversation: Uuid) -> ChatResult<Vec<String>> {
    let room = room_of_chat(chat, conversation).await?;
    Ok(room
        .map(|room| room.members.into_iter().map(|m| m.user_id).collect())
        .unwrap_or_default())
}

pub async fn gate_send(
    chat: &ChatState,
    conversation: Uuid,
    user_id: &str,
    class: &str,
) -> ChatResult<RoomChatMember> {
    let room = room_of_chat(chat, conversation)
        .await?
        .ok_or(ChatError::NotMember)?;
    let member = RoomChatMember::of(&room, user_id, now_ms()).ok_or(ChatError::NotMember)?;
    if !room.chat_enabled {
        return Err(ChatError::Unavailable);
    }
    if let Some(until) = member.muted_until {
        return Err(ChatError::Muted { until });
    }
    let slow_mode_s = room.slow_mode();
    if class == "post" && slow_mode_s > 0 && !member.can_moderate() {
        hold_slow_mode(chat, conversation, user_id, slow_mode_s).await?;
    }
    Ok(member)
}

async fn hold_slow_mode(
    chat: &ChatState,
    conversation: Uuid,
    user_id: &str,
    seconds: u32,
) -> ChatResult<()> {
    let key = rt_keys::slow_mode(conversation, user_id);
    let mut conn = chat.redis.get().await?;
    let taken: Option<String> = redis::cmd("SET")
        .arg(&key)
        .arg(1)
        .arg("NX")
        .arg("PX")
        .arg(u64::from(seconds) * 1_000)
        .query_async(&mut conn)
        .await?;
    if taken.is_some() {
        return Ok(());
    }
    let left: i64 = conn.pttl(&key).await?;
    Err(ChatError::SlowMode {
        retry_ms: u64::try_from(left).unwrap_or(0).max(1),
    })
}
