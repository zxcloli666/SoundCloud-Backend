use axum::http::StatusCode;

use crate::error::AppError;
use crate::modules::chat::ChatError;

const RETRY_MS: u64 = 500;

pub fn refusal(error: AppError) -> ChatError {
    match error {
        AppError::Coded {
            code: "room_not_member",
            ..
        } => return ChatError::NotMember,
        AppError::Coded {
            code: "room_moderators_full",
            ..
        } => return ChatError::Invalid,
        _ => {}
    }
    match error.status() {
        StatusCode::NOT_FOUND => ChatError::NotFound,
        StatusCode::FORBIDDEN => ChatError::Unavailable,
        StatusCode::BAD_REQUEST => ChatError::Invalid,
        StatusCode::CONFLICT | StatusCode::SERVICE_UNAVAILABLE | StatusCode::TOO_MANY_REQUESTS => {
            ChatError::Busy { retry_ms: RETRY_MS }
        }
        _ => match error {
            AppError::Redis(_) | AppError::RedisPool(_) => ChatError::busy(),
            other => {
                tracing::error!(error = %other, "room request failed");
                ChatError::Internal
            }
        },
    }
}
