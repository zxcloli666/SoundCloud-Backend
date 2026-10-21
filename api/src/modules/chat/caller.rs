mod cache;
mod resolve;

use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use chat_wire::prost::Message;
use chat_wire::proto::Bind;
use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::modules::chat::ChatError;
use crate::modules::chat::model::Tier;

pub use cache::{Callers, SessionEntry};
pub use resolve::{Resolved, resolve};

#[derive(Clone, Debug)]
pub struct ChatCaller {
    pub user_id: String,
    pub session_id: Uuid,
    pub device: Option<CallerDevice>,
    pub tier: Tier,
    pub max_bytes: u32,
}

#[derive(Clone, Debug)]
pub struct CallerDevice {
    pub id: Uuid,
    pub created_at: DateTime<Utc>,
    pub history_unlocked: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CallerBind {
    pub device_id: Uuid,
    pub auth_pub: [u8; 32],
    pub box_pub: [u8; 32],
}

impl CallerBind {
    pub fn from_bind(bind: &Bind) -> Option<Self> {
        Some(Self {
            device_id: Uuid::parse_str(&bind.device_id).ok()?,
            auth_pub: bind.auth_pub.as_slice().try_into().ok()?,
            box_pub: bind.box_pub.as_slice().try_into().ok()?,
        })
    }

    pub fn decode(stored: &[u8]) -> Option<Self> {
        Self::from_bind(&Bind::decode(stored).ok()?)
    }

    pub fn owns(&self, stored: &[u8]) -> bool {
        Self::decode(stored).is_some_and(|other| other == *self)
    }
}

impl<S: Send + Sync> FromRequestParts<S> for ChatCaller {
    type Rejection = ChatError;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        parts
            .extensions
            .get::<ChatCaller>()
            .cloned()
            .ok_or(ChatError::Internal)
    }
}

impl<S: Send + Sync> FromRequestParts<S> for CallerBind {
    type Rejection = ChatError;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        parts
            .extensions
            .get::<CallerBind>()
            .copied()
            .ok_or(ChatError::ReloginRequired)
    }
}
