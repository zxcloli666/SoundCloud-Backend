use axum::http::StatusCode;

use crate::error::AppError;

const RETRY_AFTER_SECONDS: i64 = 2;
const QUERY_CANCELED: &str = "57014";

#[derive(Clone, Debug)]
pub struct SearchFailure {
    code: &'static str,
    retry_after: i64,
    detail: Option<String>,
}

impl SearchFailure {
    pub fn from_app(error: AppError) -> Self {
        match from_app(error) {
            AppError::Coded {
                code,
                retry_after_sec,
                ..
            } if is_search_code(code) => Self {
                code,
                retry_after: retry_after_sec.unwrap_or(RETRY_AFTER_SECONDS),
                detail: None,
            },
            other => Self {
                code: "request_failed",
                retry_after: 0,
                detail: Some(other.to_string()),
            },
        }
    }

    pub fn into_app(self) -> AppError {
        match self.detail {
            Some(detail) => AppError::internal(detail),
            None => coded(self.code).with_retry_after(self.retry_after),
        }
    }
}

pub fn from_db(error: sqlx::Error) -> AppError {
    match &error {
        sqlx::Error::Database(db) if db.code().as_deref() == Some(QUERY_CANCELED) => {
            failed("search_timeout")
        }
        sqlx::Error::PoolTimedOut => busy(),
        _ => AppError::Db(error),
    }
}

pub fn from_app(error: AppError) -> AppError {
    match error {
        AppError::Db(error) => from_db(error),
        other => other,
    }
}

pub fn busy() -> AppError {
    failed("search_busy")
}

fn failed(code: &'static str) -> AppError {
    crate::metrics::record_search_failure(code);
    coded(code)
}

fn is_search_code(code: &str) -> bool {
    matches!(code, "search_busy" | "search_timeout")
}

fn coded(code: &'static str) -> AppError {
    let message = match code {
        "search_busy" => "Search is busy, try again shortly",
        _ => "Search took too long, try again shortly",
    };
    AppError::coded(StatusCode::SERVICE_UNAVAILABLE, code, message)
        .with_retry_after(RETRY_AFTER_SECONDS)
}
