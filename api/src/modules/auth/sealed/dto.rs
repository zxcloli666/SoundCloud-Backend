use axum::Json;
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::AppError;
use crate::modules::chat::ChatError;

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
