use axum::http::StatusCode;

use crate::error::AppError;

const RETRY_SECONDS: i64 = 30;

pub(super) fn not_linked() -> AppError {
    AppError::coded(
        StatusCode::NOT_FOUND,
        "mobile_not_linked",
        "No mobile profile is linked to this account",
    )
}

pub(super) fn relink_required() -> AppError {
    AppError::coded(
        StatusCode::CONFLICT,
        "mobile_relink_required",
        "The mobile profile must be linked again",
    )
}

pub(super) fn captcha() -> AppError {
    AppError::coded(
        StatusCode::SERVICE_UNAVAILABLE,
        "mobile_captcha",
        "SoundCloud asked for a bot check",
    )
}

pub(super) fn unavailable() -> AppError {
    AppError::coded(
        StatusCode::BAD_GATEWAY,
        "mobile_unavailable",
        "SoundCloud did not answer the mobile request",
    )
    .with_retry_after(RETRY_SECONDS)
}

pub(super) fn invalid(code: &'static str, message: &'static str) -> AppError {
    AppError::coded(StatusCode::BAD_REQUEST, code, message)
}
