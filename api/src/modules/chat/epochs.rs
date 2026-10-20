use uuid::Uuid;

use crate::modules::chat::{ChatError, ChatResult, ChatState};

pub const EPOCH_MESSAGES_MAX: i32 = 10_000;
pub const EPOCH_DAYS_MAX: i64 = 30;

pub async fn current_epoch(_chat: &ChatState, _conversation: Uuid) -> ChatResult<i32> {
    Err(ChatError::Unimplemented)
}

pub async fn mark_rekey(_chat: &ChatState, _conversation: Uuid) -> ChatResult<()> {
    Err(ChatError::Unimplemented)
}
