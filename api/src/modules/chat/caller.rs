use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::modules::chat::ChatError;
use crate::modules::chat::model::Tier;

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
