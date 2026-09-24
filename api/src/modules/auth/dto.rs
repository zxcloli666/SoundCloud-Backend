use chrono::{DateTime, NaiveDateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Serialize)]
pub struct LoginResponse {
    pub url: String,
    #[serde(rename = "loginRequestId")]
    pub login_request_id: Uuid,
}

#[derive(Debug, Deserialize)]
pub struct LoginStatusQuery {
    pub id: String,
}

#[derive(Debug, Deserialize)]
pub struct CallbackQuery {
    pub code: String,
    pub state: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionResponse {
    pub authenticated: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub soundcloud_user_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<DateTime<Utc>>,
}

#[derive(Serialize)]
pub struct LogoutResponse {
    pub success: bool,
}

#[derive(Debug, Deserialize)]
pub struct CreateLinkRequest {
    pub mode: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateLinkResponse {
    pub link_request_id: Uuid,
    pub claim_token: String,
    pub expires_at: NaiveDateTime,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaimLinkRequest {
    pub claim_token: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaimLinkResponse {
    pub session_id: Uuid,
    pub mode: String,
}

#[derive(Debug, Deserialize)]
pub struct LinkStatusQuery {
    pub id: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LinkStatusResponse {
    pub status: String,
    pub mode: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SoundCloudConnectionState {
    Ready,
    RefreshDue,
    Refreshing,
    RetryLater,
    ReauthorizationRequired,
    NotConnected,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SoundCloudConnectionResponse {
    pub state: SoundCloudConnectionState,
    pub can_use_soundcloud: bool,
    pub can_refresh: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub soundcloud_user_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub soundcloud_urn: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub access_token_expires_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_in_sec: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_attempt_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_success_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retry_after_sec: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
}
