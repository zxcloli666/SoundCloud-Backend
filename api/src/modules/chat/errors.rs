use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::{Value, json};

pub type ChatResult<T> = Result<T, ChatError>;

#[derive(Debug, Clone, PartialEq)]
pub enum ChatError {
    NotMember,
    EpochStale,
    Muted { until: i64 },
    SlowMode { retry_ms: u64 },
    Unavailable,
    RequestLimit,
    SendLimited,
    ReadLimited,
    Busy { retry_ms: u64 },
    Clock { server_now: i64 },
    Replay,
    DeviceRevoked,
    ReloginRequired,
    SessionBoundOther,
    TooYoung,
    Disabled,
    SendPaused,
    Invalid,
    NotFound,
    Unimplemented,
    Internal,
}

impl ChatError {
    pub fn code(&self) -> &'static str {
        match self {
            ChatError::NotMember => "chat_not_member",
            ChatError::EpochStale => "chat_epoch_stale",
            ChatError::Muted { .. } => "chat_muted",
            ChatError::SlowMode { .. } => "chat_slow_mode",
            ChatError::Unavailable => "chat_unavailable",
            ChatError::RequestLimit => "chat_request_limit",
            ChatError::SendLimited => "chat_send_limited",
            ChatError::ReadLimited => "chat_read_limited",
            ChatError::Busy { .. } => "chat_busy",
            ChatError::Clock { .. } => "chat_clock",
            ChatError::Replay => "chat_replay",
            ChatError::DeviceRevoked => "chat_device_revoked",
            ChatError::ReloginRequired => "chat_relogin_required",
            ChatError::SessionBoundOther => "chat_session_bound_other",
            ChatError::TooYoung => "chat_too_young",
            ChatError::Disabled => "chat_disabled",
            ChatError::SendPaused => "chat_send_paused",
            ChatError::Invalid => "chat_invalid",
            ChatError::NotFound => "chat_not_found",
            ChatError::Unimplemented => "chat_unimplemented",
            ChatError::Internal => "chat_internal",
        }
    }

    pub fn status(&self) -> StatusCode {
        match self {
            ChatError::NotMember
            | ChatError::Muted { .. }
            | ChatError::Unavailable
            | ChatError::TooYoung => StatusCode::FORBIDDEN,
            ChatError::EpochStale | ChatError::Replay | ChatError::SessionBoundOther => {
                StatusCode::CONFLICT
            }
            ChatError::SlowMode { .. }
            | ChatError::RequestLimit
            | ChatError::SendLimited
            | ChatError::ReadLimited => StatusCode::TOO_MANY_REQUESTS,
            ChatError::Clock { .. } | ChatError::DeviceRevoked | ChatError::ReloginRequired => {
                StatusCode::UNAUTHORIZED
            }
            ChatError::Busy { .. } | ChatError::Disabled | ChatError::SendPaused => {
                StatusCode::SERVICE_UNAVAILABLE
            }
            ChatError::Invalid => StatusCode::BAD_REQUEST,
            ChatError::NotFound => StatusCode::NOT_FOUND,
            ChatError::Unimplemented => StatusCode::NOT_IMPLEMENTED,
            ChatError::Internal => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    pub fn body(&self) -> Value {
        let mut body = json!({ "error": self.code() });
        match self {
            ChatError::Muted { until } => body["until"] = json!(until),
            ChatError::SlowMode { retry_ms } | ChatError::Busy { retry_ms } => {
                body["retry_ms"] = json!(retry_ms)
            }
            ChatError::Clock { server_now } => body["server_now"] = json!(server_now),
            _ => {}
        }
        body
    }
}

impl IntoResponse for ChatError {
    fn into_response(self) -> Response {
        (self.status(), Json(self.body())).into_response()
    }
}

impl From<sqlx::Error> for ChatError {
    fn from(error: sqlx::Error) -> Self {
        tracing::error!(%error, "chat query failed");
        ChatError::Internal
    }
}

impl From<deadpool_redis::PoolError> for ChatError {
    fn from(error: deadpool_redis::PoolError) -> Self {
        tracing::warn!(%error, "redis-rt pool is unavailable");
        ChatError::Busy { retry_ms: 1_000 }
    }
}

impl From<redis::RedisError> for ChatError {
    fn from(error: redis::RedisError) -> Self {
        tracing::warn!(%error, "redis-rt command failed");
        ChatError::Busy { retry_ms: 1_000 }
    }
}

#[cfg(test)]
#[path = "tests/errors_tests.rs"]
mod tests;
