use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use serde_json::Value;
use uuid::Uuid;

use crate::modules::chat::body::ChatPath;
use crate::modules::chat::{ChatCaller, ChatError, ChatResult, ChatState};

pub async fn create(
    _caller: ChatCaller,
    State(_chat): State<Arc<ChatState>>,
    ChatPath(_conversation): ChatPath<Uuid>,
) -> ChatResult<Json<Value>> {
    Err(ChatError::Unimplemented)
}

pub async fn revoke(
    _caller: ChatCaller,
    State(_chat): State<Arc<ChatState>>,
    ChatPath(_token): ChatPath<String>,
) -> ChatResult<StatusCode> {
    Err(ChatError::Unimplemented)
}

pub async fn join(
    _caller: ChatCaller,
    State(_chat): State<Arc<ChatState>>,
    ChatPath(_token): ChatPath<String>,
) -> ChatResult<Json<Value>> {
    Err(ChatError::Unimplemented)
}
