use axum::Json;
use axum::extract::FromRequestParts;
use axum::http::StatusCode;
use axum::http::request::Parts;
use axum::response::{IntoResponse, Response};
use backend_contracts::admin_catalog::{AdminCatalogRequest, AdminCommand};
use uuid::Uuid;

use crate::bus::nats::NatsRequestError;
use crate::error::{AppError, AppResult};
use crate::state::AppState;

const RETRY_AFTER_SECONDS: i64 = 5;

pub struct RequestKey(Uuid);

impl FromRequestParts<AppState> for RequestKey {
    type Rejection = AppError;

    async fn from_request_parts(parts: &mut Parts, _: &AppState) -> Result<Self, Self::Rejection> {
        let Some(value) = parts.headers.get("idempotency-key") else {
            return Ok(Self(Uuid::new_v4()));
        };
        value
            .to_str()
            .ok()
            .and_then(|value| Uuid::parse_str(value.trim()).ok())
            .map(Self)
            .ok_or_else(|| AppError::bad_request("Idempotency-Key must be a UUID"))
    }
}

pub async fn forward(
    state: &AppState,
    key: RequestKey,
    command: AdminCommand,
) -> AppResult<Response> {
    let request = AdminCatalogRequest {
        request_id: key.0,
        command,
    };
    let reply = state
        .core
        .admin_catalog(&request)
        .await
        .map_err(unreachable_core)?;
    let status = StatusCode::from_u16(reply.status).map_err(|_| {
        AppError::coded(
            StatusCode::BAD_GATEWAY,
            "core_failed",
            "core answered with a bad status",
        )
    })?;
    Ok((status, Json(reply.body)).into_response())
}

fn unreachable_core(error: NatsRequestError) -> AppError {
    let (status, code, message) = match error {
        NatsRequestError::Timeout(_) => (
            StatusCode::GATEWAY_TIMEOUT,
            "core_timeout",
            "core did not answer in time; the edit may have been applied, send it again with the same Idempotency-Key",
        ),
        NatsRequestError::NoResponders(_) | NatsRequestError::Transport(..) => (
            StatusCode::SERVICE_UNAVAILABLE,
            "core_unavailable",
            "core is unavailable, retry shortly",
        ),
        NatsRequestError::Decode(..) => (
            StatusCode::BAD_GATEWAY,
            "core_failed",
            "core failed to apply the edit",
        ),
    };
    AppError::coded(status, code, message).with_retry_after(RETRY_AFTER_SECONDS)
}
