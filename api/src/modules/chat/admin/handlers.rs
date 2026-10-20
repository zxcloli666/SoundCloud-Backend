use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use serde_json::Value;
use uuid::Uuid;

use crate::modules::chat::admin::auth::ChatAdmin;
use crate::modules::chat::body::ChatPath;
use crate::modules::chat::{ChatError, ChatResult, ChatState};

pub async fn reports(_: ChatAdmin, State(_chat): State<Arc<ChatState>>) -> ChatResult<Json<Value>> {
    Err(ChatError::Unimplemented)
}

pub async fn report(
    _: ChatAdmin,
    State(_chat): State<Arc<ChatState>>,
    ChatPath(_id): ChatPath<Uuid>,
) -> ChatResult<Json<Value>> {
    Err(ChatError::Unimplemented)
}

pub async fn resolve_report(
    _: ChatAdmin,
    State(_chat): State<Arc<ChatState>>,
    ChatPath(_id): ChatPath<Uuid>,
) -> ChatResult<Json<Value>> {
    Err(ChatError::Unimplemented)
}

pub async fn restrictions(
    _: ChatAdmin,
    State(_chat): State<Arc<ChatState>>,
) -> ChatResult<Json<Value>> {
    Err(ChatError::Unimplemented)
}

pub async fn restrict(
    _: ChatAdmin,
    State(_chat): State<Arc<ChatState>>,
) -> ChatResult<Json<Value>> {
    Err(ChatError::Unimplemented)
}

pub async fn delete_message(
    _: ChatAdmin,
    State(_chat): State<Arc<ChatState>>,
    ChatPath((_conversation, _seq)): ChatPath<(Uuid, i64)>,
) -> ChatResult<StatusCode> {
    Err(ChatError::Unimplemented)
}

pub async fn unlist(
    _: ChatAdmin,
    State(_chat): State<Arc<ChatState>>,
    ChatPath(_code): ChatPath<String>,
) -> ChatResult<StatusCode> {
    Err(ChatError::Unimplemented)
}

pub async fn rewrap(_: ChatAdmin, State(_chat): State<Arc<ChatState>>) -> ChatResult<Json<Value>> {
    Err(ChatError::Unimplemented)
}

pub async fn stats(_: ChatAdmin, State(_chat): State<Arc<ChatState>>) -> ChatResult<Json<Value>> {
    Err(ChatError::Unimplemented)
}
