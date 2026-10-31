use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

pub const AUTH_REFRESH_SUBJECT: &str = "auth.refresh.v1";
pub const AUTH_APP_TOKEN_SUBJECT: &str = "auth.app_token.v1";
pub const AUTH_RPC_QUEUE: &str = "jobs-auth-rpc-v1";

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RefreshRequest {
    pub connection_id: Uuid,
    #[serde(default)]
    pub rejected_token_fingerprint: Option<String>,
    #[serde(default)]
    pub hold: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RefreshOutcome {
    Refreshed,
    AlreadyFresh,
    InProgress,
    RateLimited,
    RetryLater,
    ReauthorizationRequired,
    TimedOut,
    NotConnected,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RefreshReply {
    pub outcome: RefreshOutcome,
    pub connection: Option<ConnectionState>,
    #[serde(default)]
    pub retry_after_sec: Option<i64>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionState {
    pub access_token: String,
    pub expires_at_unix_ms: i64,
    pub scope: String,
    pub oauth_app_id: Option<Uuid>,
    pub refresh_generation: i64,
    pub refresh_failure_count: i32,
    pub refresh_lease_id: Option<Uuid>,
    pub refresh_lease_expires_at_unix_ms: Option<i64>,
    pub last_refresh_attempt_at_unix_ms: Option<i64>,
    pub last_refresh_success_at_unix_ms: Option<i64>,
    pub last_refresh_error_kind: Option<String>,
    pub last_refresh_error: Option<String>,
    pub retry_at_unix_ms: Option<i64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppTokenRequest {
    pub oauth_app_id: Uuid,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppTokenReply {
    pub token: Option<AppToken>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppToken {
    pub generation: Uuid,
    pub access_token: String,
    pub expires_at_unix_ms: i64,
}

pub fn token_fingerprint(token: &str) -> String {
    hex::encode(&Sha256::digest(token.as_bytes())[..16])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_old_request_without_optional_fields_still_decodes() {
        let request: RefreshRequest = serde_json::from_value(serde_json::json!({
            "connectionId": Uuid::nil(),
        }))
        .expect("request decodes");

        assert!(request.rejected_token_fingerprint.is_none());
        assert!(!request.hold);
    }

    #[test]
    fn a_fingerprint_names_one_token_without_revealing_it() {
        let fingerprint = token_fingerprint("token-a");

        assert_eq!(fingerprint.len(), 32);
        assert_eq!(fingerprint, token_fingerprint("token-a"));
        assert_ne!(fingerprint, token_fingerprint("token-b"));
        assert!(!fingerprint.contains("token"));
    }

    #[test]
    fn outcomes_travel_as_snake_case() {
        assert_eq!(
            serde_json::to_value(RefreshOutcome::ReauthorizationRequired).expect("encodes"),
            serde_json::json!("reauthorization_required")
        );
    }
}
