use axum::extract::FromRequestParts;
use axum::http::request::Parts;

use crate::modules::chat::ChatError;

pub const TOKEN_FILE: &str = "chat-admin-token";
pub const TOKEN_HEADER: &str = "x-chat-admin-token";
pub const ADMIN_HEADER: &str = "x-chat-admin-user";

pub struct ChatAdmin {
    pub admin: String,
}

impl<S: Send + Sync> FromRequestParts<S> for ChatAdmin {
    type Rejection = ChatError;

    async fn from_request_parts(_parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        Err(ChatError::Unimplemented)
    }
}
