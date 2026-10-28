use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use subtle::ConstantTimeEq;

use crate::error::AppError;
use crate::state::AppState;

pub struct AdminAuth;

pub struct MetricsAuth;

impl FromRequestParts<AppState> for AdminAuth {
    type Rejection = AppError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let provided = header(parts, "x-admin-token");
        if !token_matches(provided, state.config.admin.token.expose()) {
            return Err(AppError::unauthorized("Invalid admin token"));
        }
        Ok(AdminAuth)
    }
}

impl FromRequestParts<AppState> for MetricsAuth {
    type Rejection = AppError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let bearer = header(parts, "authorization")
            .strip_prefix("Bearer ")
            .unwrap_or("");
        if token_matches(bearer, state.config.admin.metrics_token.expose()) {
            return Ok(MetricsAuth);
        }
        AdminAuth::from_request_parts(parts, state).await?;
        Ok(MetricsAuth)
    }
}

fn header<'a>(parts: &'a Parts, name: &str) -> &'a str {
    parts
        .headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
}

fn token_matches(provided: &str, expected: &str) -> bool {
    !expected.is_empty() && provided.as_bytes().ct_eq(expected.as_bytes()).unwrap_u8() == 1
}
