use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use chrono::Utc;
use serde::Serialize;
use uuid::Uuid;

use crate::modules::chat::model::{DEVICES_MAX, DISAPPEARING_TTLS, GROUP_MEMBERS_MAX};
use crate::modules::chat::sync::incarnation;
use crate::modules::chat::{ChatCaller, ChatResult, ChatState};

#[derive(Debug, Clone, Serialize)]
pub struct ChatConfigView {
    pub user_id: String,
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

impl ChatLimitsView {
    pub fn current() -> Self {
        Self {
            text_max: sc_chat_wire::msg::TEXT_MAX,
            embeds_max: sc_chat_wire::msg::EMBEDS_MAX,
            group_members_max: GROUP_MEMBERS_MAX,
            devices_max: DEVICES_MAX,
            envelope_max: sc_chat_wire::seal::MAX_ENVELOPE,
            disappearing_ttls: DISAPPEARING_TTLS,
        }
    }
}

pub async fn chat_config(
    caller: ChatCaller,
    State(chat): State<Arc<ChatState>>,
) -> ChatResult<Json<ChatConfigView>> {
    let incarnation = incarnation::current(&chat).await?;
    Ok(Json(ChatConfigView {
        enabled: chat.config.opens_for(&caller.user_id),
        access: chat.config.access.as_str(),
        send_paused: chat.config.send_paused,
        hold_max_ms: chat.config.hold_max_ms,
        server_now: Utc::now().timestamp_millis(),
        incarnation,
        ock_current: chat.ocks.current(),
        limits: ChatLimitsView::current(),
        user_id: caller.user_id,
    }))
}
