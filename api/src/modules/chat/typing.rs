use std::sync::Arc;

use axum::extract::State;
use axum::http::StatusCode;
use uuid::Uuid;

use crate::modules::chat::body::ChatPath;
use crate::modules::chat::{ChatCaller, ChatError, ChatResult, ChatState};

pub async fn typing(
    _caller: ChatCaller,
    State(_chat): State<Arc<ChatState>>,
    ChatPath(_conversation): ChatPath<Uuid>,
) -> ChatResult<StatusCode> {
    Err(ChatError::Unimplemented)
}

pub async fn typing_in(_chat: &ChatState, _conversation: Uuid) -> ChatResult<Vec<String>> {
    Err(ChatError::Unimplemented)
}
