use std::sync::Arc;

use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use uuid::Uuid;

use crate::common::uuid::parse_uuid;
use crate::error::{AppError, AppResult};
use crate::modules::auth::AuthService;
use crate::state::AppState;

/// Личность запроса. Токен SoundCloud тут НЕ материализуется: подавляющее
/// большинство ручек его не трогает — они читают нашу БД по `sc_user_id`.
/// Раньше экстрактор обновлял токен всем подряд, и приложение переставало
/// работать целиком, стоило SoundCloud затупить на рефреше. Кому токен реально
/// нужен — зовёт [`SessionCtx::access_token`] (4 ручки из ~95).
#[derive(Clone)]
pub struct SessionCtx {
    pub session_id: Uuid,
    pub sc_user_id: String,
    auth: Arc<AuthService>,
}

impl SessionCtx {
    /// Живой access-токен SoundCloud: при необходимости обновит (и только здесь
    /// запрос может подождать SC).
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

        // Намеренно `get_session`, а не `get_valid_session`: личность не зависит
        // от свежести токена, и протухший токен не должен закрывать приложение.
        let session = state
            .auth
            .get_session(session_id)
            .await?
            .ok_or_else(|| AppError::unauthorized("Session not found"))?;

        let raw = session.soundcloud_user_id.ok_or_else(|| {
            AppError::unauthorized("Session missing SoundCloud user info, please re-authenticate")
        })?;
        // Канон идентичности — bare numeric ВЕЗДЕ. На проде per-user таблицы были
        // расщеплены URN/bare (likes/followings/owned — оба варианта). Канонизируем
        // на входе → все write'ы bare; per-user РИДЫ — variant-tolerant (ANY) на
        // переходный период; бэкфилл собирает существующее. sessions.soundcloud_user_id
        // (JWT sub) и user_profiles остаются URN (write на login не трогаем) —
        // их читаем через ANY / token lookup ANY.
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
    if let Some(v) = parts
        .headers
        .get("x-session-id")
        .and_then(|v| v.to_str().ok())
    {
        if !v.is_empty() {
            return Some(v.to_string());
        }
    }
    if let Some(q) = parts.uri.query() {
        for pair in q.split('&') {
            if let Some((k, v)) = pair.split_once('=') {
                if k == "session_id" && !v.is_empty() {
                    return Some(urlencoding::decode(v).ok()?.into_owned());
                }
            }
        }
    }
    None
}
