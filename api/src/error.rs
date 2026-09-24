use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::{Map, Value, json};
use thiserror::Error;

const SC_DEADLINE_RETRY_AFTER: i64 = 15;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("bad request: {0}")]
    BadRequest(String),

    #[error("unauthorized: {0}")]
    Unauthorized(String),

    #[error("forbidden: {0}")]
    Forbidden(String),

    #[error("not found: {0}")]
    NotFound(String),

    #[error("conflict: {0}")]
    Conflict(String),

    #[error("service unavailable: {0}")]
    ServiceUnavailable(String),

    #[error("{code}: {message}")]
    Coded {
        status: u16,
        code: &'static str,
        message: String,
        retry_after_sec: Option<i64>,
    },

    #[error("SoundCloud API error (status {status})")]
    ScApi {
        status: u16,
        body: Value,
        retry_after_sec: Option<i64>,
    },

    #[error("SoundCloud API unreachable: {0}")]
    ScUnreachable(String),

    #[error("SoundCloud connection requires reauthorization")]
    SoundCloudReauthorizationRequired,

    #[error("SoundCloud connection is temporarily unavailable")]
    SoundCloudTemporarilyUnavailable { retry_after_sec: Option<i64> },

    #[error("SoundCloud token refresh timed out")]
    SoundCloudRefreshTimedOut,

    #[error("SoundCloud read exceeded its deadline")]
    ScDeadlineExceeded,

    #[error("SoundCloud token refresh is rate-limited")]
    SoundCloudRefreshRateLimited { retry_after_sec: i64 },

    #[error("database error: {0}")]
    Db(#[from] sqlx::Error),

    #[error("redis error: {0}")]
    Redis(#[from] deadpool_redis::redis::RedisError),

    #[error("redis pool error: {0}")]
    RedisPool(#[from] deadpool_redis::PoolError),

    #[error("http client error: {0}")]
    Http(#[from] wreq::Error),

    #[error("internal error: {0}")]
    Internal(String),
}

impl From<sc_transport::ScError> for AppError {
    fn from(error: sc_transport::ScError) -> Self {
        match error {
            sc_transport::ScError::Api {
                status,
                body,
                retry_after_sec,
            } => Self::ScApi {
                status,
                body,
                retry_after_sec,
            },
            sc_transport::ScError::Unreachable(detail) => Self::ScUnreachable(detail),
            sc_transport::ScError::Invalid(detail) => Self::ScUnreachable(detail),
        }
    }
}

impl From<catalog_sources::SourceError> for AppError {
    fn from(error: catalog_sources::SourceError) -> Self {
        match error {
            catalog_sources::SourceError::Status {
                status,
                body,
                retry_after_seconds,
            } => Self::ScApi {
                status,
                body: Value::String(body),
                retry_after_sec: retry_after_seconds.and_then(|value| i64::try_from(value).ok()),
            },
            catalog_sources::SourceError::Unreachable(detail) => Self::ScUnreachable(detail),
            catalog_sources::SourceError::Invalid(detail) => Self::Internal(detail),
            catalog_sources::SourceError::Oversized { limit } => {
                Self::Internal(format!("external source response exceeds {limit} bytes"))
            }
            catalog_sources::SourceError::NotConfigured(what) => {
                Self::Internal(format!("{what} is not configured"))
            }
        }
    }
}

impl AppError {
    pub fn unauthorized(msg: impl Into<String>) -> Self {
        Self::Unauthorized(msg.into())
    }

    pub fn forbidden(msg: impl Into<String>) -> Self {
        Self::Forbidden(msg.into())
    }

    pub fn bad_request(msg: impl Into<String>) -> Self {
        Self::BadRequest(msg.into())
    }

    pub fn not_found(msg: impl Into<String>) -> Self {
        Self::NotFound(msg.into())
    }

    pub fn conflict(msg: impl Into<String>) -> Self {
        Self::Conflict(msg.into())
    }

    pub fn service_unavailable(msg: impl Into<String>) -> Self {
        Self::ServiceUnavailable(msg.into())
    }

    pub fn coded(status: StatusCode, code: &'static str, msg: impl Into<String>) -> Self {
        Self::Coded {
            status: status.as_u16(),
            code,
            message: msg.into(),
            retry_after_sec: None,
        }
    }

    pub fn with_retry_after(mut self, seconds: i64) -> Self {
        if let Self::Coded {
            retry_after_sec, ..
        } = &mut self
        {
            *retry_after_sec = Some(seconds.max(1));
        }
        self
    }

    pub fn internal(msg: impl Into<String>) -> Self {
        Self::Internal(msg.into())
    }

    pub fn soundcloud_reauthorization_required() -> Self {
        Self::SoundCloudReauthorizationRequired
    }

    pub fn soundcloud_temporarily_unavailable() -> Self {
        Self::SoundCloudTemporarilyUnavailable {
            retry_after_sec: None,
        }
    }

    pub fn soundcloud_temporarily_unavailable_for(retry_after_sec: Option<i64>) -> Self {
        Self::SoundCloudTemporarilyUnavailable {
            retry_after_sec: retry_after_sec.map(|seconds| seconds.max(1)),
        }
    }

    pub fn soundcloud_refresh_timed_out() -> Self {
        Self::SoundCloudRefreshTimedOut
    }

    pub fn sc_deadline_exceeded() -> Self {
        Self::ScDeadlineExceeded
    }

    pub fn soundcloud_refresh_rate_limited(retry_after_sec: i64) -> Self {
        Self::SoundCloudRefreshRateLimited {
            retry_after_sec: retry_after_sec.max(1),
        }
    }

    pub fn status(&self) -> StatusCode {
        match self {
            Self::BadRequest(_) => StatusCode::BAD_REQUEST,
            Self::Unauthorized(_) => StatusCode::UNAUTHORIZED,
            Self::Forbidden(_) => StatusCode::FORBIDDEN,
            Self::NotFound(_) => StatusCode::NOT_FOUND,
            Self::Conflict(_) => StatusCode::CONFLICT,
            Self::ServiceUnavailable(_) => StatusCode::SERVICE_UNAVAILABLE,
            Self::Coded { status, .. } | Self::ScApi { status, .. } => {
                StatusCode::from_u16(*status).unwrap_or(StatusCode::BAD_GATEWAY)
            }
            Self::ScUnreachable(_) => StatusCode::BAD_GATEWAY,
            Self::SoundCloudReauthorizationRequired => StatusCode::CONFLICT,
            Self::SoundCloudTemporarilyUnavailable { .. } => StatusCode::BAD_GATEWAY,
            Self::SoundCloudRefreshTimedOut | Self::ScDeadlineExceeded => {
                StatusCode::GATEWAY_TIMEOUT
            }
            Self::SoundCloudRefreshRateLimited { .. } => StatusCode::TOO_MANY_REQUESTS,
            Self::Db(sqlx::Error::PoolTimedOut | sqlx::Error::PoolClosed) => {
                StatusCode::SERVICE_UNAVAILABLE
            }
            Self::RedisPool(
                deadpool_redis::PoolError::Timeout(_) | deadpool_redis::PoolError::Closed,
            ) => StatusCode::SERVICE_UNAVAILABLE,
            Self::Http(error) if error.is_timeout() => StatusCode::GATEWAY_TIMEOUT,
            Self::Db(_)
            | Self::Redis(_)
            | Self::RedisPool(_)
            | Self::Http(_)
            | Self::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let status = self.status();

        if status.is_server_error() {
            tracing::error!(error = %self, "request failed");
        } else {
            tracing::debug!(error = %self, status = %status, "request rejected");
        }

        let body = match &self {
            Self::ScApi { body, .. } => {
                let mut merged = Map::new();
                merged.insert("statusCode".into(), json!(status.as_u16()));
                merged.insert("error".into(), json!("SoundCloud API error"));
                match body {
                    Value::Object(obj) => {
                        for (k, v) in obj {
                            merged.insert(k.clone(), v.clone());
                        }
                    }
                    Value::Null => {}
                    other => {
                        merged.insert("message".into(), json!(other.to_string()));
                    }
                }
                Value::Object(merged)
            }
            Self::Coded { .. }
            | Self::SoundCloudReauthorizationRequired
            | Self::SoundCloudTemporarilyUnavailable { .. }
            | Self::SoundCloudRefreshTimedOut
            | Self::SoundCloudRefreshRateLimited { .. } => json!({
                "statusCode": status.as_u16(),
                "code": self.public_code(),
                "message": self.public_message(),
                "error": status.canonical_reason().unwrap_or("Error"),
            }),
            _ => json!({
                "statusCode": status.as_u16(),
                "message": self.public_message(),
                "error": status.canonical_reason().unwrap_or("Error"),
            }),
        };

        let mut response = (status, Json(body)).into_response();
        if let Some(retry_after_sec) = self.retry_after_sec()
            && let Ok(value) = retry_after_sec.to_string().parse()
        {
            response
                .headers_mut()
                .insert(axum::http::header::RETRY_AFTER, value);
        }
        response
    }
}

impl AppError {
    pub fn public_code(&self) -> &'static str {
        match self {
            Self::Coded { code, .. } => code,
            Self::SoundCloudReauthorizationRequired => "soundcloud_reauthorization_required",
            Self::SoundCloudTemporarilyUnavailable { .. } => "soundcloud_temporarily_unavailable",
            Self::SoundCloudRefreshTimedOut => "soundcloud_refresh_timed_out",
            Self::ScDeadlineExceeded => "soundcloud_read_timed_out",
            Self::SoundCloudRefreshRateLimited { .. } => "soundcloud_refresh_rate_limited",
            _ => "request_failed",
        }
    }

    fn public_message(&self) -> String {
        match self {
            Self::BadRequest(m)
            | Self::Unauthorized(m)
            | Self::Forbidden(m)
            | Self::NotFound(m)
            | Self::Conflict(m)
            | Self::ServiceUnavailable(m)
            | Self::Coded { message: m, .. } => m.clone(),
            Self::ScUnreachable(_) => "SoundCloud API unreachable".to_owned(),
            Self::SoundCloudReauthorizationRequired => {
                "SoundCloud connection requires reauthorization".to_owned()
            }
            Self::SoundCloudTemporarilyUnavailable { .. } => {
                "SoundCloud connection is temporarily unavailable".to_owned()
            }
            Self::SoundCloudRefreshTimedOut => "SoundCloud token refresh timed out".to_owned(),
            Self::ScDeadlineExceeded => "SoundCloud did not answer in time".to_owned(),
            Self::SoundCloudRefreshRateLimited { .. } => {
                "SoundCloud token refresh is rate-limited".to_owned()
            }
            Self::ScApi { body, .. } => body.to_string(),
            Self::Db(_)
            | Self::Redis(_)
            | Self::RedisPool(_)
            | Self::Http(_)
            | Self::Internal(_) => "Internal server error".to_string(),
        }
    }

    fn retry_after_sec(&self) -> Option<i64> {
        let seconds = match self {
            Self::ScApi {
                retry_after_sec, ..
            }
            | Self::Coded {
                retry_after_sec, ..
            }
            | Self::SoundCloudTemporarilyUnavailable { retry_after_sec } => *retry_after_sec,
            Self::SoundCloudRefreshRateLimited { retry_after_sec } => Some(*retry_after_sec),
            Self::ScDeadlineExceeded => Some(SC_DEADLINE_RETRY_AFTER),
            _ => None,
        };
        seconds.map(|seconds| seconds.clamp(1, crate::sc::RETRY_AFTER_MAX_SECONDS))
    }
}

pub type AppResult<T> = Result<T, AppError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn internal_details_are_not_public() {
        let error = AppError::internal("database host and query text");

        assert_eq!(error.public_message(), "Internal server error");
    }

    #[test]
    fn exhausted_database_pool_is_unavailable() {
        let error = AppError::Db(sqlx::Error::PoolTimedOut);

        assert_eq!(error.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[test]
    fn upstream_retry_after_is_preserved() {
        let response = AppError::ScApi {
            status: 429,
            body: Value::Null,
            retry_after_sec: Some(17),
        }
        .into_response();

        assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(
            response.headers().get(axum::http::header::RETRY_AFTER),
            Some(&axum::http::HeaderValue::from_static("17"))
        );
    }

    #[test]
    fn an_absurd_upstream_retry_after_never_reaches_our_own_clients() {
        let response = AppError::ScApi {
            status: 429,
            body: Value::Null,
            retry_after_sec: Some(10_000_000),
        }
        .into_response();

        assert_eq!(
            response.headers().get(axum::http::header::RETRY_AFTER),
            Some(&axum::http::HeaderValue::from_static("3600"))
        );
    }

    #[test]
    fn temporary_connection_error_can_carry_retry_after() {
        let response = AppError::soundcloud_temporarily_unavailable_for(Some(31)).into_response();

        assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
        assert_eq!(
            response.headers().get(axum::http::header::RETRY_AFTER),
            Some(&axum::http::HeaderValue::from_static("31"))
        );
    }

    #[tokio::test]
    async fn explicit_service_unavailability_has_a_503_response() {
        let response =
            AppError::service_unavailable("No active OAuth apps available").into_response();
        let status = response.status();
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("error response body should be readable");
        let body: Value =
            serde_json::from_slice(&body).expect("error response body should contain valid JSON");

        assert_eq!(
            (status, body["message"].as_str()),
            (
                StatusCode::SERVICE_UNAVAILABLE,
                Some("No active OAuth apps available")
            )
        );
    }
}

#[cfg(test)]
mod outage_contract_tests {
    use super::*;

    #[test]
    fn a_ban_page_instead_of_json_is_a_gateway_failure_not_our_bug() {
        let error = AppError::from(sc_transport::ScError::invalid(
            "SC JSON decode: expected value at line 1 column 1",
        ));
        assert!(matches!(error, AppError::ScUnreachable(_)));
        assert_eq!(error.status(), StatusCode::BAD_GATEWAY);
    }

    #[test]
    fn an_unreachable_upstream_and_an_unusable_body_answer_alike() {
        let unreachable = AppError::from(sc_transport::ScError::unreachable("connection reset"));
        let unusable = AppError::from(sc_transport::ScError::invalid("<html>banned</html>"));
        assert_eq!(unreachable.status(), unusable.status());
    }

    #[test]
    fn upstream_statuses_still_reach_the_client_unchanged() {
        let rate_limited = AppError::from(sc_transport::ScError::Api {
            status: 429,
            body: serde_json::Value::Null,
            retry_after_sec: Some(30),
        });
        assert_eq!(rate_limited.status(), StatusCode::TOO_MANY_REQUESTS);
    }
}
