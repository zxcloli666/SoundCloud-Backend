use std::path::Path;
use std::sync::Arc;

use axum::Json;
use axum::extract::FromRequestParts;
use axum::http::header::AUTHORIZATION;
use axum::http::request::Parts;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use serde_json::json;
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

use crate::modules::chat::ChatState;
use crate::modules::chat::moderation::AUDIT_ADMIN;

pub const TOKEN_FILE: &str = "chat-admin-token";
pub const METRICS_TOKEN_FILE: &str = "chat-metrics-token";
pub const TOKEN_HEADER: &str = "x-chat-admin-token";
pub const ADMIN_HEADER: &str = "x-chat-admin-user";
pub const ADMIN_NAME_MAX: usize = 64;
pub const UNNAMED_ADMIN: &str = "admin";

pub struct ChatAdmin {
    pub admin: String,
}

pub struct MetricsReader;

pub struct AdminRejection;

impl IntoResponse for AdminRejection {
    fn into_response(self) -> Response {
        (
            StatusCode::UNAUTHORIZED,
            Json(json!({"error": "chat_admin_unauthorized"})),
        )
            .into_response()
    }
}

impl FromRequestParts<Arc<ChatState>> for ChatAdmin {
    type Rejection = AdminRejection;

    async fn from_request_parts(
        parts: &mut Parts,
        chat: &Arc<ChatState>,
    ) -> Result<Self, Self::Rejection> {
        let expected = token_in(&chat.config.keys_dir, TOKEN_FILE).await;
        let provided = provided_token(&parts.headers);
        if !token_matches(expected.as_deref(), provided) {
            return Err(AdminRejection);
        }
        Ok(ChatAdmin {
            admin: admin_name(&parts.headers),
        })
    }
}

impl FromRequestParts<Arc<ChatState>> for MetricsReader {
    type Rejection = AdminRejection;

    async fn from_request_parts(
        parts: &mut Parts,
        chat: &Arc<ChatState>,
    ) -> Result<Self, Self::Rejection> {
        let provided = provided_token(&parts.headers);
        for file in [METRICS_TOKEN_FILE, TOKEN_FILE] {
            let expected = token_in(&chat.config.keys_dir, file).await;
            if token_matches(expected.as_deref(), provided) {
                return Ok(MetricsReader);
            }
        }
        Err(AdminRejection)
    }
}

async fn token_in(keys_dir: &Path, file: &str) -> Option<String> {
    let raw = tokio::fs::read_to_string(keys_dir.join(file)).await.ok()?;
    let token = raw.trim();
    (!token.is_empty()).then(|| token.to_owned())
}

fn provided_token(headers: &HeaderMap) -> Option<&str> {
    if let Some(value) = headers.get(TOKEN_HEADER) {
        return value.to_str().ok();
    }
    headers
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
}

pub fn token_matches(expected: Option<&str>, provided: Option<&str>) -> bool {
    let expected_digest = Sha256::digest(expected.unwrap_or_default().as_bytes());
    let provided_digest = Sha256::digest(provided.unwrap_or_default().as_bytes());
    let same: bool = expected_digest.ct_eq(&provided_digest).into();
    same & expected.is_some() & provided.is_some()
}

fn admin_name(headers: &HeaderMap) -> String {
    let named = headers
        .get(ADMIN_HEADER)
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|name| !name.is_empty() && name.len() <= ADMIN_NAME_MAX)
        .filter(|name| {
            name.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"._@-".contains(&b))
        })
        .filter(|name| *name != AUDIT_ADMIN);
    named.unwrap_or(UNNAMED_ADMIN).to_owned()
}
