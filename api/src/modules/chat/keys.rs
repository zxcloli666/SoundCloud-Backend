use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::modules::chat::body::ChatPath;
use crate::modules::chat::{ChatCaller, ChatError, ChatResult, ChatState};

#[derive(Debug, Clone, Deserialize)]
pub struct KeysQuery {
    pub from_epoch: Option<i32>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SealedKey {
    pub epoch: i32,
    #[serde(with = "crate::modules::chat::b64")]
    pub sealed: Vec<u8>,
}

pub async fn list(
    _caller: ChatCaller,
    State(_chat): State<Arc<ChatState>>,
    ChatPath(_conversation): ChatPath<Uuid>,
) -> ChatResult<Json<Vec<SealedKey>>> {
    Err(ChatError::Unimplemented)
}
