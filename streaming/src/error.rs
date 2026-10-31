use std::time::Duration;

use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("not found")]
    NotFound,
    #[error("unauthorized")]
    Unauthorized,
    #[error("forbidden")]
    Forbidden,
    #[error("no stream available")]
    NoStream,
    #[error("free hq busy")]
    FreeHqBusy(Duration),
    #[error("timed out")]
    Timeout,
    #[error("soundcloud did not answer about this track")]
    Upstream,
    #[error("internal: {0}")]
    Internal(String),
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, msg) = match &self {
            AppError::NotFound => (StatusCode::NOT_FOUND, self.to_string()),
            AppError::Unauthorized => (StatusCode::UNAUTHORIZED, self.to_string()),
            AppError::Forbidden => (StatusCode::FORBIDDEN, self.to_string()),
            AppError::NoStream => (StatusCode::NOT_FOUND, self.to_string()),
            AppError::FreeHqBusy(_) => (StatusCode::TOO_MANY_REQUESTS, self.to_string()),
            AppError::Timeout => (StatusCode::GATEWAY_TIMEOUT, self.to_string()),
            AppError::Upstream => (StatusCode::BAD_GATEWAY, self.to_string()),
            AppError::Internal(m) => (StatusCode::INTERNAL_SERVER_ERROR, m.clone()),
        };
        if matches!(
            self,
            AppError::NoStream | AppError::Timeout | AppError::FreeHqBusy(_)
        ) {
            telemetry::fail(&tracing::Span::current(), &self.to_string());
        }
        let mut response = (status, serde_json::json!({"error": msg}).to_string()).into_response();
        if let AppError::FreeHqBusy(wait) = self {
            response
                .headers_mut()
                .insert(header::RETRY_AFTER, HeaderValue::from(wait.as_secs()));
        }
        response
    }
}

impl From<crate::db::postgres::PgError> for AppError {
    fn from(e: crate::db::postgres::PgError) -> Self {
        AppError::Internal(format!("pg: {e}"))
    }
}

impl From<wreq::Error> for AppError {
    fn from(e: wreq::Error) -> Self {
        AppError::Internal(format!("http: {}", e.without_uri()))
    }
}
