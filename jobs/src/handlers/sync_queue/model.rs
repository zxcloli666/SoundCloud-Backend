use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::FromRow;
use uuid::Uuid;

#[derive(Clone, Debug, FromRow)]
pub struct ClaimedMutation {
    pub id: Uuid,
    pub user_id: String,
    pub action_type: String,
    pub target_urn: String,
    pub payload: Option<Value>,
    pub retry_count: i32,
    pub generation: i64,
    pub lease_id: Uuid,
    pub lease_generation: i64,
    pub remote_attempted_generation: Option<i64>,
    pub remote_completed_generation: Option<i64>,
    pub remote_result: Option<Value>,
}

impl ClaimedMutation {
    pub fn has_remote_result(&self) -> bool {
        self.remote_completed_generation == Some(self.generation) && self.remote_result.is_some()
    }

    pub fn has_ambiguous_remote_attempt(&self) -> bool {
        self.remote_attempted_generation == Some(self.generation) && !self.has_remote_result()
    }
}

#[derive(Debug, FromRow)]
pub struct LockedMutation {
    pub generation: i64,
    pub lease_id: Option<Uuid>,
    pub lease_generation: Option<i64>,
    pub remote_completed_generation: Option<i64>,
    pub remote_result: Option<Value>,
}

impl LockedMutation {
    pub fn is_owned_by(&self, mutation: &ClaimedMutation) -> bool {
        self.lease_id == Some(mutation.lease_id)
            && self.lease_generation == Some(mutation.lease_generation)
    }

    pub fn is_ready_to_finalize(&self, mutation: &ClaimedMutation) -> bool {
        self.is_owned_by(mutation)
            && self.generation == mutation.lease_generation
            && self.remote_completed_generation == Some(mutation.lease_generation)
            && self.remote_result.is_some()
    }
}

#[derive(Clone, FromRow)]
pub struct Connection {
    pub id: Uuid,
    pub soundcloud_user_id: String,
    pub access_token: String,
    pub refresh_token: String,
    pub expires_at: DateTime<Utc>,
    pub scope: String,
    pub refresh_generation: i64,
    pub refresh_failure_count: i32,
    pub refresh_lease_id: Option<Uuid>,
    pub refresh_lease_expires_at: Option<DateTime<Utc>>,
    pub last_refresh_error_kind: Option<String>,
    pub retry_at: Option<DateTime<Utc>>,
    pub oauth_app_id: Option<Uuid>,
    pub client_id: Option<String>,
    pub client_secret: Option<String>,
    pub app_active: Option<bool>,
}

impl Connection {
    pub fn token_is_usable(&self) -> bool {
        self.expires_at > Utc::now()
            && !matches!(
                self.last_refresh_error_kind.as_deref(),
                Some("token_rejected" | "reauthorization_required")
            )
    }

    pub fn active_refresh_lease_seconds(&self) -> Option<i64> {
        self.refresh_lease_id?;
        seconds_until(self.refresh_lease_expires_at?)
    }

    pub fn retry_seconds(&self) -> Option<i64> {
        seconds_until(self.retry_at?)
    }

    pub fn refresh_credentials(&self) -> Option<(&str, &str)> {
        if self.oauth_app_id.is_none() || self.app_active != Some(true) {
            return None;
        }
        Some((self.client_id.as_deref()?, self.client_secret.as_deref()?))
    }
}

fn seconds_until(deadline: DateTime<Utc>) -> Option<i64> {
    let milliseconds = deadline
        .signed_duration_since(Utc::now())
        .num_milliseconds();
    (milliseconds > 0).then(|| milliseconds.saturating_add(999) / 1_000)
}
