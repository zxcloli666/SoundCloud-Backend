pub mod wire;

use std::sync::Arc;

use axum::Json;
use axum::extract::State;

use crate::modules::chat::sync::wire::SyncAnswer;
use crate::modules::chat::{ChatCaller, ChatError, ChatResult, ChatState};

pub async fn sync(
    _caller: ChatCaller,
    State(_chat): State<Arc<ChatState>>,
) -> ChatResult<Json<SyncAnswer>> {
    Err(ChatError::Unimplemented)
}
