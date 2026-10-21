use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::AppError;
use crate::modules::auth::service::login::{LoginInitResult, LoginStatusResult};
use crate::modules::chat::body::{ChatJson, ChatQuery};
use crate::modules::chat::channel::sealed_auth::SealedAuth;
use crate::modules::chat::{ChatError, ChatState};

pub enum AuthReject {
    Chat(ChatError),
    App(AppError),
}

impl From<ChatError> for AuthReject {
    fn from(error: ChatError) -> Self {
        AuthReject::Chat(error)
    }
}

impl From<AppError> for AuthReject {
    fn from(error: AppError) -> Self {
        AuthReject::App(error)
    }
}

impl IntoResponse for AuthReject {
    fn into_response(self) -> Response {
        match self {
            AuthReject::Chat(error) => error.into_response(),
            AuthReject::App(error) => error.into_response(),
        }
    }
}

pub type AuthResult<T> = Result<Json<T>, AuthReject>;

#[derive(Debug, Deserialize)]
pub struct RequestId {
    pub id: Uuid,
}

#[derive(Debug, Deserialize)]
pub struct LinkCreateBody {
    pub mode: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LinkClaimBody {
    pub claim_token: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LinkCreated {
    pub link_request_id: Uuid,
    pub claim_token: String,
    pub expires_at: i64,
    pub mode: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LinkClaimed {
    pub session_id: Uuid,
    pub mode: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LinkPoll {
    pub status: String,
    pub mode: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

pub async fn login(
    auth: SealedAuth,
    State(chat): State<Arc<ChatState>>,
) -> AuthResult<LoginInitResult> {
    let started = chat
        .auth
        .initiate_bound_login(auth.session_id, &auth.bind)
        .await?;
    Ok(Json(started))
}

pub async fn login_status(
    auth: SealedAuth,
    State(chat): State<Arc<ChatState>>,
    ChatQuery(request): ChatQuery<RequestId>,
) -> AuthResult<LoginStatusResult> {
    let status = chat
        .auth
        .login_request_status_for(request.id, |stored| auth.keys.owns(stored))
        .await?;
    Ok(Json(status))
}

pub async fn link_create(
    auth: SealedAuth,
    State(chat): State<Arc<ChatState>>,
    ChatJson(body): ChatJson<LinkCreateBody>,
) -> AuthResult<LinkCreated> {
    let source = auth.session_id.filter(|_| body.mode != "pull");
    let created = chat
        .link
        .create_bound(&body.mode, source, Some(&auth.bind), |stored| {
            auth.keys.owns(stored)
        })
        .await?;
    Ok(Json(LinkCreated {
        link_request_id: created.link_request_id,
        claim_token: created.claim_token,
        expires_at: created.expires_at.and_utc().timestamp_millis(),
        mode: body.mode,
    }))
}

pub async fn link_claim(
    auth: SealedAuth,
    State(chat): State<Arc<ChatState>>,
    ChatJson(body): ChatJson<LinkClaimBody>,
) -> AuthResult<LinkClaimed> {
    let claimed = chat
        .link
        .claim_bound(
            &body.claim_token,
            auth.session_id,
            Some(&auth.bind),
            |stored| auth.keys.owns(stored),
        )
        .await?;
    Ok(Json(LinkClaimed {
        session_id: claimed.session_id,
        mode: claimed.mode,
    }))
}

pub async fn link_status(
    auth: SealedAuth,
    State(chat): State<Arc<ChatState>>,
    ChatQuery(request): ChatQuery<RequestId>,
) -> AuthResult<LinkPoll> {
    let status = chat
        .link
        .get_status_for(request.id, |stored| auth.keys.owns(stored))
        .await?;
    Ok(Json(LinkPoll {
        status: status.status,
        mode: status.mode,
        session_id: status.session_id,
        error: status.error,
    }))
}

#[cfg(test)]
#[path = "tests/auth_tests.rs"]
mod tests;
