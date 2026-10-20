use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use serde::Deserialize;
use serde_json::Value;
use uuid::Uuid;

use crate::modules::chat::body::ChatPath;
use crate::modules::chat::model::ConversationKind;
use crate::modules::chat::{ChatCaller, ChatError, ChatResult, ChatState};

pub const LIST_ROWS_MAX: usize = 200;
pub const LIST_BYTES_MAX: usize = 128 * 1024;

#[derive(Debug, Clone, Deserialize)]
pub struct CreateConversation {
    pub kind: ConversationKind,
    #[serde(default)]
    pub members: Vec<String>,
    #[serde(default, with = "crate::modules::chat::b64::option")]
    pub meta_ct: Option<Vec<u8>>,
    pub meta_epoch: Option<i32>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SetTtl {
    pub ttl_s: Option<i32>,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DeleteScope {
    Me,
    Everyone,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DeleteQuery {
    #[serde(rename = "for")]
    pub scope: DeleteScope,
}

pub async fn list(
    _caller: ChatCaller,
    State(_chat): State<Arc<ChatState>>,
) -> ChatResult<Json<Value>> {
    Err(ChatError::Unimplemented)
}

pub async fn create(
    _caller: ChatCaller,
    State(_chat): State<Arc<ChatState>>,
) -> ChatResult<Json<Value>> {
    Err(ChatError::Unimplemented)
}

pub async fn show(
    _caller: ChatCaller,
    State(_chat): State<Arc<ChatState>>,
    ChatPath(_conversation): ChatPath<Uuid>,
) -> ChatResult<Json<Value>> {
    Err(ChatError::Unimplemented)
}

pub async fn update(
    _caller: ChatCaller,
    State(_chat): State<Arc<ChatState>>,
    ChatPath(_conversation): ChatPath<Uuid>,
) -> ChatResult<Json<Value>> {
    Err(ChatError::Unimplemented)
}

pub async fn delete(
    _caller: ChatCaller,
    State(_chat): State<Arc<ChatState>>,
    ChatPath(_conversation): ChatPath<Uuid>,
) -> ChatResult<StatusCode> {
    Err(ChatError::Unimplemented)
}

pub async fn update_me(
    _caller: ChatCaller,
    State(_chat): State<Arc<ChatState>>,
    ChatPath(_conversation): ChatPath<Uuid>,
) -> ChatResult<Json<Value>> {
    Err(ChatError::Unimplemented)
}

pub async fn set_ttl(
    _caller: ChatCaller,
    State(_chat): State<Arc<ChatState>>,
    ChatPath(_conversation): ChatPath<Uuid>,
) -> ChatResult<Json<Value>> {
    Err(ChatError::Unimplemented)
}
