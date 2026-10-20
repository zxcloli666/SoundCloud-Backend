use crate::modules::chat::{ChatError, ChatResult, ChatState};

pub async fn run_once(_chat: &ChatState) -> ChatResult<u64> {
    Err(ChatError::Unimplemented)
}
