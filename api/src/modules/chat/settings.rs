use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use serde_json::Value;

use crate::modules::chat::{ChatCaller, ChatError, ChatResult, ChatState};

pub async fn show(
    _caller: ChatCaller,
    State(_chat): State<Arc<ChatState>>,
) -> ChatResult<Json<Value>> {
    Err(ChatError::Unimplemented)
}

pub async fn update(
    _caller: ChatCaller,
    State(_chat): State<Arc<ChatState>>,
) -> ChatResult<Json<Value>> {
    Err(ChatError::Unimplemented)
}
