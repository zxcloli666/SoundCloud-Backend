use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use serde::Deserialize;
use serde_json::Value;
use uuid::Uuid;

use crate::modules::chat::{ChatCaller, ChatError, ChatResult, ChatState};

pub const CONTEXT_MESSAGES: i64 = 10;
pub const EVIDENCE_KEEP_DAYS: i64 = 180;

#[derive(Debug, Clone, Deserialize)]
pub struct FileReport {
    pub conversation_id: Uuid,
    pub seq_from: i64,
    pub seq_to: i64,
    pub reason: String,
    pub comment: Option<String>,
}

pub async fn file(
    _caller: ChatCaller,
    State(_chat): State<Arc<ChatState>>,
) -> ChatResult<Json<Value>> {
    Err(ChatError::Unimplemented)
}
