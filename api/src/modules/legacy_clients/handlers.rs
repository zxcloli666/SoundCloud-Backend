use axum::extract::State;
use axum::response::Response;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::Value;

use crate::common::session::{RawSessionIdHeader, SessionCtx};
use crate::error::AppResult;
use crate::modules::auth::handlers::required_session_id;
use crate::state::AppState;

use super::{profile, refresh};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/me/cold", get(cold_profile))
        .route("/auth/refresh", post(renew_session))
}

async fn cold_profile(State(state): State<AppState>, ctx: SessionCtx) -> AppResult<Json<Value>> {
    Ok(Json(profile::read(&state.me, &ctx.sc_user_id).await?))
}

#[tracing::instrument(skip_all)]
async fn renew_session(
    State(state): State<AppState>,
    RawSessionIdHeader(raw): RawSessionIdHeader,
) -> AppResult<Response> {
    let session_id = required_session_id(raw.as_deref())?;
    refresh::answer(session_id, state.auth.refresh_soundcloud(session_id).await)
}

#[cfg(test)]
#[path = "handlers_tests.rs"]
mod tests;
