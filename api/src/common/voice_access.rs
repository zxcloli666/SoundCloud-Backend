use axum::http::StatusCode;

use crate::common::admission::AdmissionRejection;
use crate::error::{AppError, AppResult};
use crate::state::AppState;

pub fn admission_error(rejection: AdmissionRejection, code: &'static str, what: &str) -> AppError {
    match rejection {
        AdmissionRejection::Limited {
            retry_after_seconds,
        } => AppError::coded(
            StatusCode::TOO_MANY_REQUESTS,
            code,
            format!("Too many {what} requests"),
        )
        .with_retry_after(i64::try_from(retry_after_seconds).unwrap_or(60)),
        AdmissionRejection::Unavailable => AppError::coded(
            StatusCode::SERVICE_UNAVAILABLE,
            "voice_busy",
            format!("{what} is busy, try again shortly"),
        )
        .with_retry_after(1),
    }
}

pub async fn require_star(st: &AppState, user: &str) -> AppResult<()> {
    if st.subscriptions.is_premium_cached(user).await? {
        Ok(())
    } else {
        Err(AppError::coded(
            StatusCode::FORBIDDEN,
            "star_required",
            "Star subscription required",
        ))
    }
}
