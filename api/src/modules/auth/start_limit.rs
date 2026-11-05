use axum::http::StatusCode;
use sqlx::postgres::PgDatabaseError;

use crate::error::AppError;

pub const STARTS_LIMITED: &str = "auth_starts_limited";
pub const STARTS_RETRY_AFTER_SECONDS: i64 = 60;

pub fn starts_limited(error: sqlx::Error) -> AppError {
    let Some(database) = error.as_database_error() else {
        return AppError::from(error);
    };
    if database.code().as_deref() != Some("SC429") {
        return AppError::from(error);
    }
    let retry_ms = database
        .try_downcast_ref::<PgDatabaseError>()
        .and_then(PgDatabaseError::detail)
        .and_then(|detail| detail.parse::<i64>().ok());
    let retry_after = retry_ms.map_or(STARTS_RETRY_AFTER_SECONDS, |ms| (ms + 999) / 1000);
    AppError::coded(
        StatusCode::TOO_MANY_REQUESTS,
        STARTS_LIMITED,
        "Too many sign-ins are starting right now, retry shortly",
    )
    .with_retry_after(retry_after.clamp(1, STARTS_RETRY_AFTER_SECONDS))
}
