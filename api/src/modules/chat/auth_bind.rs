use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use serde_json::Value;

use crate::modules::chat::{ChatError, ChatResult, ChatState};

pub async fn login(State(_chat): State<Arc<ChatState>>) -> ChatResult<Json<Value>> {
    Err(ChatError::Unimplemented)
}

pub async fn login_status(State(_chat): State<Arc<ChatState>>) -> ChatResult<Json<Value>> {
    Err(ChatError::Unimplemented)
}

pub async fn link_create(State(_chat): State<Arc<ChatState>>) -> ChatResult<Json<Value>> {
    Err(ChatError::Unimplemented)
}

pub async fn link_claim(State(_chat): State<Arc<ChatState>>) -> ChatResult<Json<Value>> {
    Err(ChatError::Unimplemented)
}

pub async fn link_status(State(_chat): State<Arc<ChatState>>) -> ChatResult<Json<Value>> {
    Err(ChatError::Unimplemented)
}
