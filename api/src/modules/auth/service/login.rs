use std::time::Duration;

use base64::Engine;
use chrono::Utc;
use rand::RngCore;
use serde_json::Value;
use sha2::{Digest, Sha256};
use sqlx::{Postgres, Transaction};
use tracing::{error, info, warn};
use uuid::Uuid;

use crate::error::{AppError, AppResult};
use crate::modules::auth::TokenKind;
use crate::modules::auth::model::{LoginRequest, SoundCloudConnection};
use crate::sc::{ScMe, ScReadService, ScTokenResponse};

use super::AuthService;
use super::oauth::{
    access_token_expiry, access_token_subject, oauth_app_failure_cooldown, public_error_message,
};

pub(super) const LOGIN_REQUEST_TTL_SECS: i64 = 15 * 60;
pub(super) const MAX_AUTH_RETRIES: i32 = 3;
const CALLBACK_TIMEOUT: Duration = Duration::from_secs(25);
const PROFILE_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, serde::Serialize)]
pub struct LoginInitResult {
    pub url: String,
    #[serde(rename = "loginRequestId")]
    pub login_request_id: Uuid,
}

#[derive(Debug, Clone)]
pub struct CallbackResult {
    pub login_request_id: Option<Uuid>,
    pub initial_status: String,
    pub username: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct LoginStatusResult {
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub step: Option<String>,
    #[serde(rename = "sessionId", skip_serializing_if = "Option::is_none")]
    pub session_id: Option<Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(rename = "redirectUrl", skip_serializing_if = "Option::is_none")]
    pub redirect_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extract: Option<String>,
}

struct StoredConnection {
    connection: SoundCloudConnection,
    orphan_candidate: Option<Uuid>,
}

struct ConnectionCredentials<'a> {
    target_session_id: Option<Uuid>,
    soundcloud_user_id: &'a str,
    username: Option<&'a str>,
    oauth_app_id: Uuid,
    token: &'a ScTokenResponse,
    expires_at: chrono::DateTime<Utc>,
}

impl AuthService {
    pub async fn initiate_login(
        &self,
        existing_session_id: Option<Uuid>,
    ) -> AppResult<LoginInitResult> {
        let code_verifier = base64_url(&random_bytes(32));
        let code_challenge = base64_url(Sha256::digest(code_verifier.as_bytes()).as_slice());
        let state = hex::encode(random_bytes(16));
        let (credentials, oauth_app_id) = self.pick_credentials(None).await?;
        let target_session_id = match existing_session_id {
            Some(session_id) if self.get_session(session_id).await?.is_some() => Some(session_id),
            Some(_) => {
                warn!("Re-auth target does not exist");
                None
            }
            None => None,
        };
        let expires_at =
            (Utc::now() + chrono::Duration::seconds(LOGIN_REQUEST_TTL_SECS)).naive_utc();
        let login_request_id = Uuid::new_v4();
        let oauth_app_id = oauth_app_id.to_string();

        sqlx::query(
            "INSERT INTO login_requests \
                (id, state, code_verifier, oauth_app_id, target_session_id, status, expires_at) \
             VALUES ($1, $2, $3, $4, $5, 'pending', $6)",
        )
        .bind(login_request_id)
        .bind(&state)
        .bind(&code_verifier)
        .bind(&oauth_app_id)
        .bind(target_session_id)
        .bind(expires_at)
        .execute(&self.pool)
        .await?;

        Ok(LoginInitResult {
            url: self.build_authorize_url(&credentials, &state, &code_challenge)?,
            login_request_id,
        })
    }

    pub async fn handle_callback(
        &self,
        public: &ScReadService,
        code: &str,
        state: &str,
    ) -> AppResult<CallbackResult> {
        let claimed = sqlx::query_file_as!(
            LoginRequest,
            "queries/auth/service/claim_login_request.sql",
            state
        )
        .fetch_optional(&self.pool)
        .await?;

        if let Some(login_request) = claimed {
            let request_id = login_request.id;
            match tokio::time::timeout(
                CALLBACK_TIMEOUT,
                self.complete_login(public, login_request, code.to_owned()),
            )
            .await
            {
                Ok(Ok(())) => {}
                Ok(Err(error)) => {
                    error!(%error, "OAuth callback failed");
                    let message =
                        public_error_message(&error, "Authentication failed. Please try again.");
                    self.mark_request_failed(request_id, &message).await?;
                }
                Err(_) => {
                    warn!("OAuth callback timed out");
                    self.mark_request_failed(
                        request_id,
                        "SoundCloud authentication timed out. Please try again.",
                    )
                    .await?;
                }
            }
            let status = self.get_login_request_status(request_id).await?;
            return Ok(CallbackResult {
                login_request_id: Some(request_id),
                initial_status: status.status,
                username: status.username,
                error: status.error,
            });
        }

        let existing = sqlx::query_file_as!(
            LoginRequest,
            "queries/auth/service/get_login_request_by_state.sql",
            state
        )
        .fetch_optional(&self.pool)
        .await?;
        let Some(existing) = existing else {
            return Ok(CallbackResult {
                login_request_id: None,
                initial_status: "failed".to_owned(),
                username: None,
                error: Some("This login link is invalid or already used.".to_owned()),
            });
        };

        let initial_status = match existing.status.as_str() {
            "completed" => "completed",
            "processing" => "pending",
            _ => "failed",
        };
        Ok(CallbackResult {
            login_request_id: Some(existing.id),
            initial_status: initial_status.to_owned(),
            username: existing.username,
            error: existing.error,
        })
    }

    async fn complete_login(
        &self,
        public: &ScReadService,
        login_request: LoginRequest,
        code: String,
    ) -> AppResult<()> {
        if login_request.expires_at < Utc::now().naive_utc() {
            self.mark_request_failed(login_request.id, "Login request expired")
                .await?;
            return Ok(());
        }

        let (credentials, oauth_app_id) = self
            .credentials_for_login_app(login_request.oauth_app_id.as_deref())
            .await?;
        let app_key = crate::modules::auth::health::oauth_app_key(&credentials.client_id);
        let token = match self
            .health
            .run_oauth_request(
                oauth_app_id,
                self.sc
                    .exchange_code_for_token(&code, &login_request.code_verifier, &credentials),
            )
            .await
        {
            Ok(token) => token,
            Err(error) => {
                let message = public_error_message(&error, "Token exchange failed");
                if let Some(minimum_cooldown) = oauth_app_failure_cooldown(&error) {
                    crate::metrics::record_sc_failure(&error);
                    self.retry_with_new_app(
                        &login_request,
                        &credentials.client_id,
                        &message,
                        minimum_cooldown,
                    )
                    .await?;
                } else {
                    self.mark_request_failed(login_request.id, &message).await?;
                }
                return Ok(());
            }
        };
        if token.access_token.is_empty() || token.refresh_token.is_empty() {
            self.mark_request_failed(
                login_request.id,
                "SoundCloud returned an incomplete token response",
            )
            .await?;
            return Ok(());
        }

        let _ = sqlx::query_file!(
            "queries/auth/service/login_request_step_extract.sql",
            login_request.id
        )
        .execute(&self.pool)
        .await;

        let observation = catalog_ingest::Observation::begin(&self.pool).await?;
        let mut profile = None;
        let soundcloud_urn = match access_token_subject(&token.access_token) {
            Some(subject) => subject,
            None => match self.fetch_sc_me(&token.access_token).await {
                MeOutcome::Found(me, value) => {
                    profile = Some(value);
                    me.urn
                }
                MeOutcome::Unauthorized => {
                    self.mark_request_failed(login_request.id, "SoundCloud rejected the token")
                        .await?;
                    return Ok(());
                }
                MeOutcome::Unreachable => {
                    self.mark_request_failed(
                        login_request.id,
                        "Failed to identify the SoundCloud account",
                    )
                    .await?;
                    return Ok(());
                }
            },
        };
        let soundcloud_user_id = crate::common::sc_ids::extract_sc_id(&soundcloud_urn).to_owned();
        if soundcloud_user_id.is_empty() {
            self.mark_request_failed(login_request.id, "SoundCloud account id is empty")
                .await?;
            return Ok(());
        }

        if profile.is_none() {
            profile = self
                .fetch_profile(public, &token.access_token, &soundcloud_user_id)
                .await;
        }
        profile = profile.and_then(|value| normalize_login_profile(value, &soundcloud_user_id));
        let username = profile
            .as_ref()
            .and_then(|value| value.get("username"))
            .and_then(Value::as_str)
            .map(str::to_owned);
        let profile_ok = profile.is_some();

        let _ = sqlx::query_file!(
            "queries/auth/service/login_request_step_finalizing.sql",
            login_request.id
        )
        .execute(&self.pool)
        .await;

        let expires_at = access_token_expiry(&token.access_token)
            .unwrap_or_else(|| Utc::now() + chrono::Duration::seconds(token.expires_in.max(1)));
        let mut transaction = self.pool.begin().await?;

        if let Some(profile) = profile.as_ref() {
            catalog_ingest::upsert_profile_in(
                &mut transaction,
                &soundcloud_user_id,
                profile,
                observation,
            )
            .await?;
        }

        let stored = self
            .store_connection(
                &mut transaction,
                ConnectionCredentials {
                    target_session_id: login_request.target_session_id,
                    soundcloud_user_id: &soundcloud_user_id,
                    username: username.as_deref(),
                    oauth_app_id,
                    token: &token,
                    expires_at,
                },
            )
            .await?;
        let session_id = self
            .store_session(
                &mut transaction,
                login_request.target_session_id,
                stored.connection.id,
            )
            .await?;

        if let Some(connection_id) = stored.orphan_candidate {
            sqlx::query_file!(
                "queries/auth/service/delete_orphan_connection.sql",
                connection_id
            )
            .execute(&mut *transaction)
            .await?;
        }

        let completed = sqlx::query_file!(
            "queries/auth/service/login_request_completed.sql",
            login_request.id,
            session_id,
            username,
            profile_ok
        )
        .fetch_optional(&mut *transaction)
        .await?;
        if completed.is_none() {
            return Err(AppError::internal(
                "login request disappeared while finalizing",
            ));
        }
        transaction.commit().await?;

        let _ = self.health.record_app_success(&app_key).await;
        info!("Login completed");
        Ok(())
    }

    async fn store_connection(
        &self,
        transaction: &mut Transaction<'_, Postgres>,
        credentials: ConnectionCredentials<'_>,
    ) -> AppResult<StoredConnection> {
        let ConnectionCredentials {
            target_session_id,
            soundcloud_user_id,
            username,
            oauth_app_id,
            token,
            expires_at,
        } = credentials;
        let target = if let Some(session_id) = target_session_id {
            sqlx::query_file!(
                "queries/auth/service/lock_session_connection.sql",
                session_id
            )
            .fetch_optional(&mut **transaction)
            .await?
        } else {
            None
        };

        let previous_connection_id = target.as_ref().and_then(|row| row.soundcloud_connection_id);
        let reusable_connection_id = if let Some(connection_id) = previous_connection_id {
            let current_user = sqlx::query_file_scalar!(
                "queries/auth/service/lock_connection_user.sql",
                connection_id
            )
            .fetch_optional(&mut **transaction)
            .await?;
            current_user
                .as_deref()
                .filter(|current| *current == soundcloud_user_id)
                .map(|_| connection_id)
        } else {
            None
        };

        let username = username.unwrap_or_default();
        if let Some(connection_id) = reusable_connection_id {
            let connection = sqlx::query_file_as!(
                SoundCloudConnection,
                "queries/auth/service/update_connection.sql",
                connection_id,
                soundcloud_user_id,
                username,
                oauth_app_id,
                &token.access_token,
                &token.refresh_token,
                expires_at,
                &token.scope
            )
            .fetch_one(&mut **transaction)
            .await?;
            return Ok(StoredConnection {
                connection,
                orphan_candidate: None,
            });
        }

        let connection = sqlx::query_file_as!(
            SoundCloudConnection,
            "queries/auth/service/insert_connection.sql",
            Uuid::now_v7(),
            soundcloud_user_id,
            username,
            oauth_app_id,
            &token.access_token,
            &token.refresh_token,
            expires_at,
            &token.scope
        )
        .fetch_one(&mut **transaction)
        .await?;
        Ok(StoredConnection {
            connection,
            orphan_candidate: previous_connection_id,
        })
    }

    async fn store_session(
        &self,
        transaction: &mut Transaction<'_, Postgres>,
        target_session_id: Option<Uuid>,
        connection_id: Uuid,
    ) -> AppResult<Uuid> {
        if let Some(session_id) = target_session_id {
            let updated = sqlx::query_file!(
                "queries/auth/service/attach_session.sql",
                session_id,
                connection_id
            )
            .fetch_optional(&mut **transaction)
            .await?;
            if updated.is_some() {
                return Ok(session_id);
            }
        }

        let session_id = Uuid::new_v4();
        sqlx::query_file!(
            "queries/auth/service/insert_session.sql",
            session_id,
            connection_id
        )
        .fetch_one(&mut **transaction)
        .await?;
        Ok(session_id)
    }

    pub async fn get_login_request_status(
        &self,
        login_request_id: Uuid,
    ) -> AppResult<LoginStatusResult> {
        let login_request = sqlx::query_file_as!(
            LoginRequest,
            "queries/auth/service/get_login_request.sql",
            login_request_id
        )
        .fetch_optional(&self.pool)
        .await?;
        let Some(login_request) = login_request else {
            return Ok(expired_login_status("Unknown login request"));
        };
        if matches!(login_request.status.as_str(), "pending" | "processing")
            && login_request.expires_at < Utc::now().naive_utc()
        {
            return Ok(expired_login_status("Login request expired"));
        }

        Ok(LoginStatusResult {
            status: if login_request.status == "processing" {
                "pending".to_owned()
            } else {
                login_request.status
            },
            step: login_request.step,
            session_id: login_request.result_session_id,
            username: login_request.username,
            error: login_request.error,
            redirect_url: login_request.redirect_url,
            extract: login_request
                .profile_ok
                .map(|success| if success { "ok" } else { "failed" }.to_owned()),
        })
    }

    pub(super) async fn mark_request_failed(&self, id: Uuid, error: &str) -> AppResult<()> {
        sqlx::query_file!("queries/auth/service/login_request_failed.sql", id, error)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn fetch_sc_me(&self, access_token: &str) -> MeOutcome {
        for attempt in 0..3 {
            match self
                .sc
                .api_get::<Value>("/me", access_token, None)
                .await
                .map_err(AppError::from)
                .and_then(|value| {
                    serde_json::from_value::<ScMe>(value.clone())
                        .map(|me| (me, value))
                        .map_err(|_| AppError::internal("Invalid SoundCloud account response"))
                }) {
                Ok((me, value)) => return MeOutcome::Found(me, value),
                Err(AppError::ScApi {
                    status: 401 | 403, ..
                }) => {
                    return MeOutcome::Unauthorized;
                }
                Err(error) if attempt < 2 => {
                    warn!(attempt, %error, "SoundCloud profile request failed");
                    tokio::time::sleep(Duration::from_millis(200 * (attempt + 1))).await;
                }
                Err(_) => return MeOutcome::Unreachable,
            }
        }
        MeOutcome::Unreachable
    }

    async fn fetch_profile(
        &self,
        public: &ScReadService,
        token: &str,
        user_id: &str,
    ) -> Option<Value> {
        type ProfileFuture<'a> =
            std::pin::Pin<Box<dyn std::future::Future<Output = AppResult<Value>> + Send + 'a>>;
        let me: ProfileFuture<'_> = Box::pin(async {
            self.sc
                .api_get::<Value>("/me", token, None)
                .await
                .map_err(AppError::from)
        });
        let public: ProfileFuture<'_> =
            Box::pin(async { public.user_by_id(TokenKind::PublicPool, user_id).await });
        tokio::time::timeout(
            PROFILE_TIMEOUT,
            futures::future::select_ok(vec![me, public]),
        )
        .await
        .ok()
        .and_then(Result::ok)
        .map(|(value, _)| value)
    }
}

pub(super) fn random_bytes(length: usize) -> Vec<u8> {
    let mut bytes = vec![0; length];
    rand::thread_rng().fill_bytes(&mut bytes);
    bytes
}

pub(super) fn base64_url(bytes: &[u8]) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

fn expired_login_status(message: &str) -> LoginStatusResult {
    LoginStatusResult {
        status: "expired".to_owned(),
        step: None,
        session_id: None,
        username: None,
        error: Some(message.to_owned()),
        redirect_url: None,
        extract: None,
    }
}

enum MeOutcome {
    Found(ScMe, Value),
    Unauthorized,
    Unreachable,
}

fn normalize_login_profile(mut value: Value, user_id: &str) -> Option<Value> {
    let object = value.as_object_mut()?;
    let urn = catalog_ingest::user_urn(user_id);
    let has_urn = object.get("urn").and_then(Value::as_str) == Some(urn.as_str());
    let has_id = object
        .get("id")
        .and_then(Value::as_i64)
        .map(|id| id.to_string())
        .as_deref()
        == Some(user_id);
    if (!has_urn && !has_id)
        || (object.contains_key("urn") && !has_urn)
        || (object.get("id").is_some_and(|id| !id.is_null()) && !has_id)
        || object
            .get("username")
            .and_then(Value::as_str)
            .is_none_or(|name| name.trim().is_empty())
    {
        return None;
    }
    object.insert("urn".into(), Value::String(urn));
    Some(value)
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};
    use sqlx::PgPool;

    use super::*;

    fn sc_error(status: u16, body: Value, retry_after_sec: Option<i64>) -> AppError {
        AppError::ScApi {
            status,
            body,
            retry_after_sec,
        }
    }

    #[test]
    fn login_keeps_missing_or_invalid_profile_data_optional() {
        let valid = json!({"id": 17, "username": "Alice"});
        let normalized = normalize_login_profile(valid, "17").unwrap();
        assert_eq!(normalized["urn"], "soundcloud:users:17");
        for invalid in [
            Value::Null,
            json!({"id": 17}),
            json!({"id": 18, "username": "Other"}),
            json!({"id": 17, "urn": "soundcloud:users:18", "username": "Other"}),
            json!({"id": 17, "username": "   "}),
        ] {
            assert!(normalize_login_profile(invalid, "17").is_none());
        }
    }

    #[test]
    fn invalid_authorization_code_does_not_penalize_oauth_app() {
        let invalid_grant = sc_error(400, json!({ "error": "invalid_grant" }), None);
        let malformed_request = sc_error(400, json!({ "error": "invalid_request" }), None);

        assert_eq!(oauth_app_failure_cooldown(&invalid_grant), None);
        assert_eq!(oauth_app_failure_cooldown(&malformed_request), None);
    }

    #[test]
    fn app_wide_failures_rotate_oauth_app() {
        let invalid_client = sc_error(401, json!({ "error": "invalid_client" }), None);
        let rate_limited = sc_error(429, Value::Null, Some(75));
        let server_failure = sc_error(503, Value::Null, None);

        assert_eq!(oauth_app_failure_cooldown(&invalid_client), Some(30 * 60));
        assert_eq!(oauth_app_failure_cooldown(&rate_limited), Some(75));
        assert_eq!(oauth_app_failure_cooldown(&server_failure), Some(30));
        assert_eq!(
            oauth_app_failure_cooldown(&AppError::ScUnreachable("offline".to_owned())),
            Some(30)
        );
    }

    #[test]
    fn internal_failures_do_not_poison_oauth_app_health() {
        assert_eq!(
            oauth_app_failure_cooldown(&AppError::internal("database unavailable")),
            None
        );
    }

    #[sqlx::test(migrations = false)]
    async fn expired_login_request_cannot_be_claimed(pool: PgPool) -> anyhow::Result<()> {
        sqlx::query(
            "CREATE TABLE login_requests (
                id uuid PRIMARY KEY,
                state text UNIQUE NOT NULL,
                code_verifier text NOT NULL,
                oauth_app_id text,
                target_session_id uuid,
                status varchar(16) NOT NULL DEFAULT 'pending',
                step varchar(16),
                username text,
                result_session_id uuid,
                error text,
                retry_count integer NOT NULL DEFAULT 0,
                redirect_url text,
                profile_ok boolean,
                expires_at timestamp NOT NULL
            )",
        )
        .execute(&pool)
        .await?;
        let request_id = Uuid::now_v7();
        sqlx::query(
            "INSERT INTO login_requests (id, state, code_verifier, expires_at)
             VALUES ($1, 'expired-state', 'verifier', now() - interval '1 second')",
        )
        .bind(request_id)
        .execute(&pool)
        .await?;

        let claimed = sqlx::query_file_as!(
            LoginRequest,
            "queries/auth/service/claim_login_request.sql",
            "expired-state"
        )
        .fetch_optional(&pool)
        .await?;
        let status: String = sqlx::query_scalar("SELECT status FROM login_requests WHERE id = $1")
            .bind(request_id)
            .fetch_one(&pool)
            .await?;

        assert!(claimed.is_none());
        assert_eq!(status, "pending");
        Ok(())
    }

    #[sqlx::test(migrations = false)]
    async fn late_failure_cannot_overwrite_completed_login(pool: PgPool) -> anyhow::Result<()> {
        sqlx::query(
            "CREATE TABLE login_requests (
                id uuid PRIMARY KEY,
                status varchar(16) NOT NULL,
                step varchar(16),
                error text
            )",
        )
        .execute(&pool)
        .await?;
        let request_id = Uuid::now_v7();
        sqlx::query("INSERT INTO login_requests (id, status) VALUES ($1, 'completed')")
            .bind(request_id)
            .execute(&pool)
            .await?;

        sqlx::query_file!(
            "queries/auth/service/login_request_failed.sql",
            request_id,
            "late timeout"
        )
        .execute(&pool)
        .await?;
        let state: (String, Option<String>) =
            sqlx::query_as("SELECT status, error FROM login_requests WHERE id = $1")
                .bind(request_id)
                .fetch_one(&pool)
                .await?;

        assert_eq!(state, ("completed".to_owned(), None));
        Ok(())
    }

    #[sqlx::test(migrations = false)]
    async fn terminal_login_request_cannot_be_retried_or_advanced(
        pool: PgPool,
    ) -> anyhow::Result<()> {
        sqlx::query(
            "CREATE TABLE login_requests (
                id uuid PRIMARY KEY,
                state text NOT NULL,
                code_verifier text NOT NULL,
                oauth_app_id text,
                status varchar(16) NOT NULL,
                step varchar(16),
                error text,
                retry_count integer NOT NULL DEFAULT 0,
                redirect_url text,
                expires_at timestamp NOT NULL
            )",
        )
        .execute(&pool)
        .await?;
        let completed = Uuid::now_v7();
        let failed = Uuid::now_v7();
        sqlx::query(
            "INSERT INTO login_requests (
                 id, state, code_verifier, status, expires_at
             ) VALUES
                 ($1, 'completed-state', 'verifier', 'completed', now()),
                 ($2, 'failed-state', 'verifier', 'failed', now())",
        )
        .bind(completed)
        .bind(failed)
        .execute(&pool)
        .await?;

        for request_id in [completed, failed] {
            let retried = sqlx::query_file_scalar!(
                "queries/auth/service/retry_login_request.sql",
                request_id,
                "new-state",
                "new-verifier",
                "new-app",
                "https://example.com/retry",
                Utc::now().naive_utc()
            )
            .fetch_optional(&pool)
            .await?;
            sqlx::query_file!(
                "queries/auth/service/login_request_step_extract.sql",
                request_id
            )
            .execute(&pool)
            .await?;
            sqlx::query_file!(
                "queries/auth/service/login_request_step_finalizing.sql",
                request_id
            )
            .execute(&pool)
            .await?;

            assert!(retried.is_none());
        }

        let states: Vec<(String, Option<String>, i32)> =
            sqlx::query_as("SELECT status, step, retry_count FROM login_requests ORDER BY status")
                .fetch_all(&pool)
                .await?;
        assert_eq!(
            states,
            vec![
                ("completed".to_owned(), None, 0),
                ("failed".to_owned(), None, 0),
            ]
        );
        Ok(())
    }
}
