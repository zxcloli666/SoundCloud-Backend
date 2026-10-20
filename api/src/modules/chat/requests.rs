use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use serde_json::Value;

use crate::modules::chat::{ChatCaller, ChatError, ChatResult, ChatState};

pub const MESSAGES_BEFORE_ACCEPT: i64 = 3;

pub async fn list(
    _caller: ChatCaller,
    State(_chat): State<Arc<ChatState>>,
) -> ChatResult<Json<Value>> {
    Err(ChatError::Unimplemented)
}
