use crate::modules::chat::{ChatError, ChatResult, ChatState};

pub async fn mark_seen(_chat: &ChatState, _user_id: &str) -> ChatResult<()> {
    Err(ChatError::Unimplemented)
}

pub async fn online_contacts(_chat: &ChatState, _user_id: &str) -> ChatResult<Vec<String>> {
    Err(ChatError::Unimplemented)
}
