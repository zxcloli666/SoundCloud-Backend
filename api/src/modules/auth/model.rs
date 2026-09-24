use chrono::{DateTime, NaiveDateTime, Utc};
use sqlx::FromRow;
use uuid::Uuid;

#[derive(Debug, Clone, FromRow)]
pub struct Session {
    pub soundcloud_connection_id: Option<Uuid>,
    pub soundcloud_user_id: Option<String>,
}

#[derive(Debug, Clone, FromRow)]
pub struct AuthSession {
    pub id: Uuid,
    pub soundcloud_connection_id: Option<Uuid>,
    pub soundcloud_user_id: Option<String>,
    pub username: Option<String>,
    pub oauth_app_id: Option<Uuid>,
    pub oauth_app_active: Option<bool>,
    pub expires_at: Option<DateTime<Utc>>,
    pub refresh_lease_id: Option<Uuid>,
    pub refresh_lease_expires_at: Option<DateTime<Utc>>,
    pub last_refresh_attempt_at: Option<DateTime<Utc>>,
    pub last_refresh_success_at: Option<DateTime<Utc>>,
    pub last_refresh_error_kind: Option<String>,
    pub last_refresh_error: Option<String>,
    pub retry_at: Option<DateTime<Utc>>,
}

impl AuthSession {
    pub fn connection_status(&self) -> Option<SoundCloudConnectionStatus> {
        self.soundcloud_connection_id?;
        Some(SoundCloudConnectionStatus {
            soundcloud_user_id: self.soundcloud_user_id.clone()?,
            has_refresh_credentials: self.oauth_app_id.is_some()
                && self.oauth_app_active == Some(true),
            expires_at: self.expires_at?,
            refresh_lease_id: self.refresh_lease_id,
            refresh_lease_expires_at: self.refresh_lease_expires_at,
            last_refresh_attempt_at: self.last_refresh_attempt_at,
            last_refresh_success_at: self.last_refresh_success_at,
            last_refresh_error_kind: self.last_refresh_error_kind.clone(),
            last_refresh_error: self.last_refresh_error.clone(),
            retry_at: self.retry_at,
        })
    }
}

#[derive(Clone, FromRow)]
pub struct SoundCloudConnection {
    pub id: Uuid,
    pub soundcloud_user_id: String,
    pub oauth_app_id: Option<Uuid>,
    pub access_token: String,
    pub refresh_token: String,
    pub expires_at: DateTime<Utc>,
    pub scope: String,
    pub refresh_generation: i64,
    pub refresh_failure_count: i32,
    pub refresh_lease_id: Option<Uuid>,
    pub refresh_lease_expires_at: Option<DateTime<Utc>>,
    pub last_refresh_attempt_at: Option<DateTime<Utc>>,
    pub last_refresh_success_at: Option<DateTime<Utc>>,
    pub last_refresh_error_kind: Option<String>,
    pub last_refresh_error: Option<String>,
    pub retry_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
pub struct SoundCloudConnectionStatus {
    pub soundcloud_user_id: String,
    pub has_refresh_credentials: bool,
    pub expires_at: DateTime<Utc>,
    pub refresh_lease_id: Option<Uuid>,
    pub refresh_lease_expires_at: Option<DateTime<Utc>>,
    pub last_refresh_attempt_at: Option<DateTime<Utc>>,
    pub last_refresh_success_at: Option<DateTime<Utc>>,
    pub last_refresh_error_kind: Option<String>,
    pub last_refresh_error: Option<String>,
    pub retry_at: Option<DateTime<Utc>>,
}

impl From<&SoundCloudConnection> for SoundCloudConnectionStatus {
    fn from(connection: &SoundCloudConnection) -> Self {
        Self {
            soundcloud_user_id: connection.soundcloud_user_id.clone(),
            has_refresh_credentials: connection.oauth_app_id.is_some(),
            expires_at: connection.expires_at,
            refresh_lease_id: connection.refresh_lease_id,
            refresh_lease_expires_at: connection.refresh_lease_expires_at,
            last_refresh_attempt_at: connection.last_refresh_attempt_at,
            last_refresh_success_at: connection.last_refresh_success_at,
            last_refresh_error_kind: connection.last_refresh_error_kind.clone(),
            last_refresh_error: connection.last_refresh_error.clone(),
            retry_at: connection.retry_at,
        }
    }
}

#[derive(Debug, Clone, FromRow)]
pub struct LoginRequest {
    pub id: Uuid,
    pub code_verifier: String,
    pub oauth_app_id: Option<String>,
    pub target_session_id: Option<Uuid>,
    pub status: String,
    pub step: Option<String>,
    pub username: Option<String>,
    pub result_session_id: Option<Uuid>,
    pub error: Option<String>,
    pub retry_count: i32,
    pub redirect_url: Option<String>,
    pub profile_ok: Option<bool>,
    pub expires_at: NaiveDateTime,
}

#[derive(Debug, Clone, FromRow)]
pub struct LinkRequestRow {
    pub id: Uuid,
    pub mode: String,
    pub source_session_id: Option<Uuid>,
    pub target_session_id: Option<Uuid>,
    pub status: String,
    pub error: Option<String>,
    pub expires_at: NaiveDateTime,
}
