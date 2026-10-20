use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use serde::Deserialize;
use uuid::Uuid;

use crate::modules::chat::body::ChatPath;
use crate::modules::chat::model::{DeviceRow, DeviceView};
use crate::modules::chat::{ChatCaller, ChatError, ChatResult, ChatState};

#[derive(Debug, Clone, Deserialize)]
pub struct RegisterDevice {
    pub platform: String,
    pub name: String,
    pub app_version: String,
    #[serde(default)]
    pub caps: Vec<String>,
}

pub async fn register(
    _caller: ChatCaller,
    State(_chat): State<Arc<ChatState>>,
) -> ChatResult<Json<DeviceView>> {
    Err(ChatError::Unimplemented)
}

pub async fn list(
    _caller: ChatCaller,
    State(_chat): State<Arc<ChatState>>,
) -> ChatResult<Json<Vec<DeviceView>>> {
    Err(ChatError::Unimplemented)
}

pub async fn remove(
    _caller: ChatCaller,
    State(_chat): State<Arc<ChatState>>,
    ChatPath(_id): ChatPath<Uuid>,
) -> ChatResult<StatusCode> {
    Err(ChatError::Unimplemented)
}

pub async fn remove_me(
    _caller: ChatCaller,
    State(_chat): State<Arc<ChatState>>,
) -> ChatResult<StatusCode> {
    Err(ChatError::Unimplemented)
}

pub async fn confirm(
    _caller: ChatCaller,
    State(_chat): State<Arc<ChatState>>,
    ChatPath(_id): ChatPath<Uuid>,
) -> ChatResult<StatusCode> {
    Err(ChatError::Unimplemented)
}

pub async fn not_me(
    _caller: ChatCaller,
    State(_chat): State<Arc<ChatState>>,
    ChatPath(_id): ChatPath<Uuid>,
) -> ChatResult<StatusCode> {
    Err(ChatError::Unimplemented)
}

pub async fn lookup(_chat: &ChatState, _id: Uuid) -> ChatResult<Option<DeviceRow>> {
    Err(ChatError::Unimplemented)
}
