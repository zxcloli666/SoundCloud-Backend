use uuid::Uuid;

use crate::modules::chat::caller::CallerDevice;
use crate::modules::chat::{ChatError, ChatResult, ChatState};

pub const UNLOCK_AFTER_HOURS: i64 = 72;
pub const ALERT_HOURS: [i64; 3] = [0, 24, 48];

pub async fn first_visible_epoch(
    _chat: &ChatState,
    _device: &CallerDevice,
    _conversation: Uuid,
) -> ChatResult<i32> {
    Err(ChatError::Unimplemented)
}
