use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use serde::Deserialize;
use serde_json::Value;

use crate::modules::chat::admin::auth::ChatAdmin;
use crate::modules::chat::{ChatError, ChatResult, ChatState};

pub const USER_PREFIX: &str = "lt:";

#[derive(Debug, Clone, Deserialize)]
pub struct Seed {
    pub users: u32,
    pub devices_per_user: u32,
    pub groups: u32,
}

pub async fn seed(_: ChatAdmin, State(chat): State<Arc<ChatState>>) -> ChatResult<Json<Value>> {
    if !chat.config.loadtest {
        return Err(ChatError::NotFound);
    }
    Err(ChatError::Unimplemented)
}

pub async fn purge(_: ChatAdmin, State(chat): State<Arc<ChatState>>) -> ChatResult<Json<Value>> {
    if !chat.config.loadtest {
        return Err(ChatError::NotFound);
    }
    Err(ChatError::Unimplemented)
}
