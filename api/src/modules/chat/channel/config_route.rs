use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use serde::Serialize;
use uuid::Uuid;

use crate::modules::chat::{ChatCaller, ChatError, ChatResult, ChatState};

#[derive(Debug, Clone, Serialize)]
pub struct ChatConfigView {
    pub enabled: bool,
    pub access: &'static str,
    pub send_paused: bool,
    pub hold_max_ms: u64,
    pub server_now: i64,
    pub incarnation: Uuid,
    pub ock_current: u8,
    pub limits: ChatLimitsView,
}

#[derive(Debug, Clone, Serialize)]
pub struct ChatLimitsView {
    pub text_max: usize,
    pub embeds_max: usize,
    pub group_members_max: usize,
    pub devices_max: usize,
    pub envelope_max: usize,
    pub disappearing_ttls: [i32; 3],
}

pub async fn chat_config(
    _caller: ChatCaller,
    State(_chat): State<Arc<ChatState>>,
) -> ChatResult<Json<ChatConfigView>> {
    Err(ChatError::Unimplemented)
}
