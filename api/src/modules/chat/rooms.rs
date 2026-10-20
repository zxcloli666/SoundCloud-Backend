use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use serde_json::Value;
use uuid::Uuid;

use crate::modules::chat::body::ChatPath;
use crate::modules::chat::{ChatCaller, ChatError, ChatResult, ChatState};

pub async fn chat(
    _caller: ChatCaller,
    State(_chat): State<Arc<ChatState>>,
    ChatPath(_code): ChatPath<String>,
) -> ChatResult<Json<Value>> {
    Err(ChatError::Unimplemented)
}

pub async fn preview(
    _caller: ChatCaller,
    State(_chat): State<Arc<ChatState>>,
    ChatPath(_code): ChatPath<String>,
) -> ChatResult<Json<Value>> {
    Err(ChatError::Unimplemented)
}

pub async fn remove_member(
    _caller: ChatCaller,
    State(_chat): State<Arc<ChatState>>,
    ChatPath((_code, _user)): ChatPath<(String, String)>,
) -> ChatResult<StatusCode> {
    Err(ChatError::Unimplemented)
}

pub async fn mute(
    _caller: ChatCaller,
    State(_chat): State<Arc<ChatState>>,
    ChatPath((_code, _user)): ChatPath<(String, String)>,
) -> ChatResult<StatusCode> {
    Err(ChatError::Unimplemented)
}

pub async fn moderator(
    _caller: ChatCaller,
    State(_chat): State<Arc<ChatState>>,
    ChatPath((_code, _user)): ChatPath<(String, String)>,
) -> ChatResult<StatusCode> {
    Err(ChatError::Unimplemented)
}

pub async fn settings(
    _caller: ChatCaller,
    State(_chat): State<Arc<ChatState>>,
    ChatPath(_code): ChatPath<String>,
) -> ChatResult<Json<Value>> {
    Err(ChatError::Unimplemented)
}

pub async fn suggest(
    _caller: ChatCaller,
    State(_chat): State<Arc<ChatState>>,
    ChatPath(_code): ChatPath<String>,
) -> ChatResult<StatusCode> {
    Err(ChatError::Unimplemented)
}

pub async fn chat_of_room(_chat: &ChatState, _code: &str) -> ChatResult<Option<Uuid>> {
    Err(ChatError::Unimplemented)
}
