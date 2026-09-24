use std::sync::Arc;

use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use uuid::Uuid;

use crate::common::uuid::parse_uuid;
use crate::error::{AppError, AppResult};
use crate::modules::auth::AuthService;
use crate::state::AppState;

#[derive(Clone)]
pub struct SessionCtx {
    pub session_id: Uuid,
    pub sc_user_id: String,
    auth: Arc<AuthService>,
}

impl SessionCtx {
    pub async fn access_token(&self) -> AppResult<String> {
        self.auth.get_valid_access_token(self.session_id).await
    }
}

impl FromRequestParts<AppState> for SessionCtx {
    type Rejection = AppError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let raw = extract_session_id(parts)
            .ok_or_else(|| AppError::unauthorized("Missing or malformed x-session-id header"))?;

        let session_id = parse_uuid(&raw)
            .ok_or_else(|| AppError::unauthorized("Missing or malformed x-session-id header"))?;

        let session = state
            .auth
            .get_session(session_id)
            .await?
            .ok_or_else(|| AppError::unauthorized("Session not found"))?;

        let raw = session.soundcloud_user_id.ok_or_else(|| {
            AppError::unauthorized("Session missing SoundCloud user info, please re-authenticate")
        })?;
        let sc_user_id = crate::common::sc_ids::extract_sc_id(&raw).to_string();

        Ok(SessionCtx {
            session_id,
            sc_user_id,
            auth: state.auth.clone(),
        })
    }
}

#[derive(Clone, Default)]
pub struct OptionalSession(pub Option<SessionCtx>);

impl FromRequestParts<AppState> for OptionalSession {
    type Rejection = AppError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let Some(raw) = extract_session_id(parts) else {
            return Ok(OptionalSession(None));
        };
        let Some(session_id) = parse_uuid(&raw) else {
            return Ok(OptionalSession(None));
        };
        let Ok(Some(session)) = state.auth.get_session(session_id).await else {
            return Ok(OptionalSession(None));
        };
        let Some(raw) = session.soundcloud_user_id else {
            return Ok(OptionalSession(None));
        };
        let sc_user_id = crate::common::sc_ids::extract_sc_id(&raw).to_string();
        Ok(OptionalSession(Some(SessionCtx {
            session_id,
            sc_user_id,
            auth: state.auth.clone(),
        })))
    }
}

pub struct RawSessionIdHeader(pub Option<String>);

impl FromRequestParts<AppState> for RawSessionIdHeader {
    type Rejection = AppError;

    async fn from_request_parts(
        parts: &mut Parts,
        _state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        Ok(RawSessionIdHeader(extract_session_id(parts)))
    }
}

fn extract_session_id(parts: &Parts) -> Option<String> {
    parts
        .headers
        .get("x-session-id")
        .and_then(|value| value.to_str().ok())
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}
