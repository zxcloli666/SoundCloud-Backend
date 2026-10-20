use std::sync::Arc;

use axum::extract::State;
use axum::http::StatusCode;

use crate::modules::chat::body::ChatPath;
use crate::modules::chat::{ChatCaller, ChatError, ChatResult, ChatState};

pub async fn block(
    _caller: ChatCaller,
    State(_chat): State<Arc<ChatState>>,
    ChatPath(_user): ChatPath<String>,
) -> ChatResult<StatusCode> {
    Err(ChatError::Unimplemented)
}

pub async fn unblock(
    _caller: ChatCaller,
    State(_chat): State<Arc<ChatState>>,
    ChatPath(_user): ChatPath<String>,
) -> ChatResult<StatusCode> {
    Err(ChatError::Unimplemented)
}

pub async fn blocked_between(_chat: &ChatState, _user: &str, _other: &str) -> ChatResult<bool> {
    Err(ChatError::Unimplemented)
}
