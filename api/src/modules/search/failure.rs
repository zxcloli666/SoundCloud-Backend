use axum::http::StatusCode;

use crate::error::AppError;

pub fn vibe_unavailable() -> AppError {
    AppError::coded(
        StatusCode::SERVICE_UNAVAILABLE,
        "vibe_unavailable",
        "Vibe search is unavailable right now",
    )
    .with_retry_after(10)
}
