use crate::modules::chat::{ChatError, ChatResult, ChatState};

pub const BATCH_ROWS: i64 = 5_000;
pub const EVERY_SECS: u64 = 60;

pub async fn run_once(_chat: &ChatState) -> ChatResult<u64> {
    Err(ChatError::Unimplemented)
}
