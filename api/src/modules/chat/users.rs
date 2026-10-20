use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use serde_json::Value;

use crate::modules::chat::body::ChatPath;
use crate::modules::chat::{ChatCaller, ChatError, ChatResult, ChatState};

pub async fn card(
    _caller: ChatCaller,
    State(_chat): State<Arc<ChatState>>,
    ChatPath(_user): ChatPath<String>,
) -> ChatResult<Json<Value>> {
    Err(ChatError::Unimplemented)
}
