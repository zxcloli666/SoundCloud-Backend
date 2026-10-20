use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use serde_json::Value;
use uuid::Uuid;

use crate::modules::chat::body::ChatPath;
use crate::modules::chat::{ChatCaller, ChatError, ChatResult, ChatState};

pub async fn list(
    _caller: ChatCaller,
    State(_chat): State<Arc<ChatState>>,
    ChatPath(_conversation): ChatPath<Uuid>,
) -> ChatResult<Json<Value>> {
    Err(ChatError::Unimplemented)
}

pub async fn add(
    _caller: ChatCaller,
    State(_chat): State<Arc<ChatState>>,
    ChatPath(_conversation): ChatPath<Uuid>,
) -> ChatResult<Json<Value>> {
    Err(ChatError::Unimplemented)
}

pub async fn remove(
    _caller: ChatCaller,
    State(_chat): State<Arc<ChatState>>,
    ChatPath((_conversation, _user)): ChatPath<(Uuid, String)>,
) -> ChatResult<StatusCode> {
    Err(ChatError::Unimplemented)
}

pub async fn update(
    _caller: ChatCaller,
    State(_chat): State<Arc<ChatState>>,
    ChatPath((_conversation, _user)): ChatPath<(Uuid, String)>,
) -> ChatResult<Json<Value>> {
    Err(ChatError::Unimplemented)
}

pub async fn accept(
    _caller: ChatCaller,
    State(_chat): State<Arc<ChatState>>,
    ChatPath(_conversation): ChatPath<Uuid>,
) -> ChatResult<StatusCode> {
    Err(ChatError::Unimplemented)
}

pub async fn decline(
    _caller: ChatCaller,
    State(_chat): State<Arc<ChatState>>,
    ChatPath(_conversation): ChatPath<Uuid>,
) -> ChatResult<StatusCode> {
    Err(ChatError::Unimplemented)
}
