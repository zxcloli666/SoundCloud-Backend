use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use chrono::NaiveDateTime;
use serde::Serialize;
use uuid::Uuid;

use crate::error::{AppError, AppResult};
use crate::modules::auth::handlers::refresh_response;
use crate::modules::auth::service::{RefreshAttempt, RefreshOutcome};

pub(super) fn answer(
    session_id: Uuid,
    refreshed: AppResult<RefreshAttempt>,
) -> AppResult<Response> {
    let attempt = refreshed.map_err(for_old_client)?;
    match attempt.outcome {
        RefreshOutcome::Refreshed | RefreshOutcome::AlreadyFresh => {
            Ok(renewed(StatusCode::OK, session_id, &attempt))
        }
        RefreshOutcome::InProgress => Ok(renewed(StatusCode::ACCEPTED, session_id, &attempt)),
        RefreshOutcome::ReauthorizationRequired => Err(reauthorization_required()),
        RefreshOutcome::NotConnected => Err(not_connected()),
        RefreshOutcome::RateLimited | RefreshOutcome::RetryLater | RefreshOutcome::TimedOut => {
            Ok(refresh_response(attempt))
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RenewedSession {
    session_id: Uuid,
    expires_at: Option<NaiveDateTime>,
}

fn renewed(status: StatusCode, session_id: Uuid, attempt: &RefreshAttempt) -> Response {
    let body = RenewedSession {
        session_id,
        expires_at: attempt
            .connection
            .as_ref()
            .map(|connection| connection.expires_at.naive_utc()),
    };
    (status, Json(body)).into_response()
}

fn for_old_client(error: AppError) -> AppError {
    match error {
        AppError::SoundCloudReauthorizationRequired => reauthorization_required(),
        other => other,
    }
}

fn reauthorization_required() -> AppError {
    AppError::coded(
        StatusCode::UNAUTHORIZED,
        "soundcloud_reauthorization_required",
        "SoundCloud rejected the refresh token, sign in again",
    )
}

fn not_connected() -> AppError {
    AppError::coded(
        StatusCode::UNAUTHORIZED,
        "soundcloud_not_connected",
        "No SoundCloud account is connected to this session, sign in again",
    )
}

#[cfg(test)]
#[path = "refresh_tests.rs"]
mod tests;
