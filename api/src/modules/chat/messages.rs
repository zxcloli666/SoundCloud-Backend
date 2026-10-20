use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::modules::chat::body::ChatPath;
use crate::modules::chat::model::MessageView;
use crate::modules::chat::{ChatCaller, ChatError, ChatResult, ChatState};

pub const PAGE_ROWS_MAX: usize = 200;
pub const PAGE_BYTES_MAX: usize = 256 * 1024;
pub const PAGE_BYTES_MAX_PRO: usize = 24 * 1024;
pub const EDIT_WINDOW_HOURS: i64 = 48;

#[derive(Debug, Clone, Deserialize)]
pub struct SendMessage {
    pub client_id: Uuid,
    pub class: String,
    pub epoch: i32,
    pub target_seq: Option<i64>,
    #[serde(with = "crate::modules::chat::b64")]
    pub body: Vec<u8>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SendAnswer {
    pub seq: i64,
    pub ts: i64,
    pub expires_at: Option<i64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct HistoryQuery {
    pub after: Option<i64>,
    pub before: Option<i64>,
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, Serialize)]
pub struct HistoryPage {
    pub messages: Vec<MessageView>,
    pub more: bool,
}

pub async fn send(
    _caller: ChatCaller,
    State(_chat): State<Arc<ChatState>>,
    ChatPath(_conversation): ChatPath<Uuid>,
) -> ChatResult<Json<SendAnswer>> {
    Err(ChatError::Unimplemented)
}

pub async fn history(
    _caller: ChatCaller,
    State(_chat): State<Arc<ChatState>>,
    ChatPath(_conversation): ChatPath<Uuid>,
) -> ChatResult<Json<HistoryPage>> {
    Err(ChatError::Unimplemented)
}

pub async fn delete(
    _caller: ChatCaller,
    State(_chat): State<Arc<ChatState>>,
    ChatPath((_conversation, _seq)): ChatPath<(Uuid, i64)>,
) -> ChatResult<StatusCode> {
    Err(ChatError::Unimplemented)
}
