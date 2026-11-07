use std::time::Duration;

use uuid::Uuid;

use super::{Charge, MINUTE, Window, charge, refund_key};
use crate::modules::chat::{ChatError, ChatResult, ChatState};

const SEAL_OVERHEAD: usize = sc_chat_wire::seal::NONCE_LEN + 16;
pub const ROOM_NEWCOMER_SECS: i64 = 600;
pub const ROOM_NEWCOMER_PLAINTEXT_MAX: usize = 2 * 256;
pub const ROOM_NEWCOMER_BODY_MAX: usize = SEAL_OVERHEAD + ROOM_NEWCOMER_PLAINTEXT_MAX;
pub const ROOM_BYTES_PER_MINUTE: u64 = 24 * 1024;

const BACKDROP_CHANGES: Window = Window::new("10m", 6, Duration::from_secs(600));
const ROOM_METAS: Window = Window::new("10m", 12, Duration::from_secs(600));

pub struct RoomPost {
    pub conversation: Uuid,
    pub public: bool,
    pub member_for_secs: i64,
    pub body_len: usize,
}

pub async fn admit_room_post(chat: &ChatState, user_id: &str, post: &RoomPost) -> ChatResult<()> {
    if !post.public {
        return Ok(());
    }
    if post.member_for_secs < ROOM_NEWCOMER_SECS && post.body_len > ROOM_NEWCOMER_BODY_MAX {
        metrics::counter!("chat_limit_rejects_total", "bucket" => "room_newcomer").increment(1);
        return Err(ChatError::SendLimited);
    }
    let budget = Window::new("bytes:1m", ROOM_BYTES_PER_MINUTE, MINUTE);
    let charges = vec![Charge::of(
        &format!("room:{}", post.conversation),
        &budget,
        user_id,
        post.body_len as u64,
    )];
    charge(chat, "room_bytes", charges, ChatError::SendLimited).await
}

pub async fn admit_backdrop(chat: &ChatState, room: &str) -> ChatResult<()> {
    let charges = vec![Charge::of("backdrop", &BACKDROP_CHANGES, room, 1)];
    charge(chat, "backdrop", charges, ChatError::RequestLimit).await
}

pub async fn refund_backdrop(chat: &ChatState, room: &str) {
    let key = Charge::of("backdrop", &BACKDROP_CHANGES, room, 1).key;
    if let Err(error) = refund_key(chat, &key).await {
        tracing::warn!(
            code = error.code(),
            "a refused backdrop change kept its charge"
        );
    }
}

pub async fn admit_room_meta(chat: &ChatState, conversation: Uuid) -> ChatResult<()> {
    let charges = vec![Charge::of(
        "room_meta",
        &ROOM_METAS,
        &conversation.to_string(),
        1,
    )];
    charge(chat, "room_meta", charges, ChatError::SendLimited).await
}
