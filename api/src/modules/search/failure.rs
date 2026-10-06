use axum::http::StatusCode;

use crate::error::AppError;

const STATEMENT_TIMEOUT: &str = "57014";

pub fn vibe_unavailable() -> AppError {
    AppError::coded(
        StatusCode::SERVICE_UNAVAILABLE,
        "vibe_unavailable",
        "Vibe search is unavailable right now",
    )
    .with_retry_after(10)
}

pub fn search_timeout() -> AppError {
    AppError::coded(
        StatusCode::SERVICE_UNAVAILABLE,
        "search_timeout",
        "Search took too long, try again",
    )
    .with_retry_after(2)
}

pub fn search_busy() -> AppError {
    AppError::coded(
        StatusCode::SERVICE_UNAVAILABLE,
        "search_busy",
        "Search is busy, try again",
    )
    .with_retry_after(2)
}

pub fn map(error: AppError) -> AppError {
    match error {
        AppError::Db(sqlx::Error::Database(db))
            if db.code().as_deref() == Some(STATEMENT_TIMEOUT) =>
        {
            search_timeout()
        }
        AppError::Db(sqlx::Error::PoolTimedOut) => search_busy(),
        other => other,
    }
}
