use crate::modules::chat::{ChatError, ChatResult, ChatState};

pub async fn hold(_chat: &ChatState) -> ChatResult<bool> {
    Err(ChatError::Unimplemented)
}
