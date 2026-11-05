use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::modules::auth::service::replica::SESSION_RETRY_AFTER_SECONDS;

pub type ChatResult<T> = Result<T, ChatError>;

pub const BUSY_TOTAL: &str = "chat_busy_total";
pub const BUSY_RETRY_MS: u64 = 1_000;
pub const BUSY_JITTER_MS: u64 = 2_000;
pub const SESSION_RETRY_MS: i64 = SESSION_RETRY_AFTER_SECONDS * 1_000;

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
    Busy { retry_ms: u64, source: BusySource },
    Clock { server_now: i64 },
    Replay,
    DeviceRevoked,
    ReloginRequired,
    SessionNotReplicated,
    SessionBoundOther,
    TooYoung,
    Disabled,
    SendPaused,
    QuotaExceeded { used: u64, quota: u64 },
    MediaStarRequired,
    RoomStarRequired,
    MediaUploadsPaused,
    MediaMissing { ids: Vec<Uuid> },
    BackdropCooldown { until: u64 },
    MediaGone,
    Invalid,
    NotFound,
    Internal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BusySource {
    ColdSession,
    ColdDevice,
    RedisPool,
    RedisCommand,
    PgPool,
    S3,
    AuthGate,
    Rooms,
    ConversationCreate,
    BackdropLock,
    StarCheck,
    MediaReserve,
    MediaCommit,
    LoadtestLease,
    AdminRooms,
    Channel,
}

impl BusySource {
    pub fn label(self) -> &'static str {
        match self {
            BusySource::ColdSession => "cold_session",
            BusySource::ColdDevice => "cold_device",
            BusySource::RedisPool => "redis_pool",
            BusySource::RedisCommand => "redis_command",
            BusySource::PgPool => "pg_pool",
            BusySource::S3 => "s3",
            BusySource::AuthGate => "auth_gate",
            BusySource::Rooms => "rooms",
            BusySource::ConversationCreate => "conversation_create",
            BusySource::BackdropLock => "backdrop_lock",
            BusySource::StarCheck => "star_check",
            BusySource::MediaReserve => "media_reserve",
            BusySource::MediaCommit => "media_commit",
            BusySource::LoadtestLease => "loadtest_lease",
            BusySource::AdminRooms => "admin_rooms",
            BusySource::Channel => "channel",
        }
    }

    pub fn count(self) {
        metrics::counter!(BUSY_TOTAL, "source" => self.label()).increment(1);
    }
}

impl ChatError {
    pub fn busy(source: BusySource) -> Self {
        ChatError::Busy {
            retry_ms: BUSY_RETRY_MS + rand::random::<u64>() % BUSY_JITTER_MS,
            source,
        }
    }

    pub fn is_busy(&self) -> bool {
        matches!(self, ChatError::Busy { .. })
    }

    pub fn count_busy(&self) {
        if let ChatError::Busy { source, .. } = self {
            source.count();
        }
    }

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
            ChatError::SessionNotReplicated => "chat_session_not_replicated",
            ChatError::SessionBoundOther => "chat_session_bound_other",
            ChatError::TooYoung => "chat_too_young",
            ChatError::Disabled => "chat_disabled",
            ChatError::SendPaused => "chat_send_paused",
            ChatError::QuotaExceeded { .. } => "chat_quota_exceeded",
            ChatError::MediaStarRequired => "chat_media_star_required",
            ChatError::RoomStarRequired => "chat_room_star_required",
            ChatError::MediaUploadsPaused => "chat_media_uploads_paused",
            ChatError::MediaMissing { .. } => "chat_media_missing",
            ChatError::BackdropCooldown { .. } => "chat_backdrop_cooldown",
            ChatError::MediaGone => "chat_media_gone",
            ChatError::Invalid => "chat_invalid",
            ChatError::NotFound => "chat_not_found",
            ChatError::Internal => "chat_internal",
        }
    }

    pub fn status(&self) -> StatusCode {
        match self {
            ChatError::NotMember
            | ChatError::Muted { .. }
            | ChatError::Unavailable
            | ChatError::TooYoung
            | ChatError::QuotaExceeded { .. }
            | ChatError::MediaStarRequired
            | ChatError::RoomStarRequired
            | ChatError::BackdropCooldown { .. } => StatusCode::FORBIDDEN,
            ChatError::MediaGone => StatusCode::GONE,
            ChatError::EpochStale
            | ChatError::Replay
            | ChatError::SessionBoundOther
            | ChatError::MediaMissing { .. } => StatusCode::CONFLICT,
            ChatError::SlowMode { .. }
            | ChatError::RequestLimit
            | ChatError::SendLimited
            | ChatError::ReadLimited => StatusCode::TOO_MANY_REQUESTS,
            ChatError::Clock { .. } | ChatError::DeviceRevoked | ChatError::ReloginRequired => {
                StatusCode::UNAUTHORIZED
            }
            ChatError::Busy { .. }
            | ChatError::SessionNotReplicated
            | ChatError::Disabled
            | ChatError::SendPaused
            | ChatError::MediaUploadsPaused => StatusCode::SERVICE_UNAVAILABLE,
            ChatError::Invalid => StatusCode::BAD_REQUEST,
            ChatError::NotFound => StatusCode::NOT_FOUND,
            ChatError::Internal => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    pub fn body(&self) -> Value {
        let mut body = json!({ "error": self.code() });
        match self {
            ChatError::Muted { until } => body["until"] = json!(until),
            ChatError::SlowMode { retry_ms } | ChatError::Busy { retry_ms, .. } => {
                body["retry_ms"] = json!(retry_ms)
            }
            ChatError::Clock { server_now } => body["server_now"] = json!(server_now),
            ChatError::SessionNotReplicated => body["retry_ms"] = json!(SESSION_RETRY_MS),
            ChatError::QuotaExceeded { used, quota } => {
                body["used"] = json!(used);
                body["quota"] = json!(quota);
            }
            ChatError::MediaMissing { ids } => body["ids"] = json!(ids),
            ChatError::BackdropCooldown { until } => body["until"] = json!(until),
            _ => {}
        }
        body
    }
}

impl IntoResponse for ChatError {
    fn into_response(self) -> Response {
        self.count_busy();
        (self.status(), Json(self.body())).into_response()
    }
}

impl From<sqlx::Error> for ChatError {
    fn from(error: sqlx::Error) -> Self {
        if matches!(error, sqlx::Error::PoolTimedOut) {
            tracing::warn!("the chat pool had no free connection in time");
            return ChatError::busy(BusySource::PgPool);
        }
        tracing::error!(%error, "chat query failed");
        ChatError::Internal
    }
}

impl From<deadpool_redis::PoolError> for ChatError {
    fn from(error: deadpool_redis::PoolError) -> Self {
        tracing::warn!(%error, "redis-rt pool is unavailable");
        ChatError::busy(BusySource::RedisPool)
    }
}

impl From<redis::RedisError> for ChatError {
    fn from(error: redis::RedisError) -> Self {
        tracing::warn!(%error, "redis-rt command failed");
        ChatError::busy(BusySource::RedisCommand)
    }
}

#[cfg(test)]
#[path = "tests/errors_tests.rs"]
mod tests;
