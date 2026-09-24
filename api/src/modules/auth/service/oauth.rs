use base64::Engine;
use chrono::{DateTime, Utc};
use sha2::Digest;
use tracing::{info, warn};
use uuid::Uuid;

use crate::error::{AppError, AppResult};
use crate::modules::auth::health::oauth_app_key;
use crate::modules::auth::model::LoginRequest;
use crate::modules::oauth_apps::model::OAuthApp;
use crate::sc::OAuthCredentials;

use super::AuthService;
use super::login::{LOGIN_REQUEST_TTL_SECS, MAX_AUTH_RETRIES, base64_url, random_bytes};

impl AuthService {
    pub(super) async fn pick_credentials(
        &self,
        exclude_client_id: Option<&str>,
    ) -> AppResult<(OAuthCredentials, Uuid)> {
        self.pick_healthy_app(exclude_client_id)
            .await
            .map(credentials_from_app)
    }

    pub(super) async fn credentials_for_login_app(
        &self,
        oauth_app_id: Option<&str>,
    ) -> AppResult<(OAuthCredentials, Uuid)> {
        let app_id = oauth_app_id
            .ok_or_else(|| AppError::not_found("OAuth app for this login is unavailable"))
            .and_then(|id| {
                Uuid::parse_str(id)
                    .map_err(|_| AppError::internal("login request has malformed OAuth app id"))
            })?;
        let credentials = self.credentials_for_app(app_id).await?;
        Ok((credentials, app_id))
    }

    pub(super) async fn credentials_for_connection(
        &self,
        connection: &crate::modules::auth::model::SoundCloudConnection,
    ) -> AppResult<OAuthCredentials> {
        let app_id = connection
            .oauth_app_id
            .ok_or_else(|| AppError::not_found("OAuth app for this connection is unavailable"))?;
        self.credentials_for_app(app_id).await
    }

    async fn credentials_for_app(&self, app_id: Uuid) -> AppResult<OAuthCredentials> {
        let app = self
            .oauth_apps
            .get_by_id(&app_id.to_string())
            .await?
            .ok_or_else(|| AppError::not_found("OAuth app is unavailable"))?;
        if !app.active {
            return Err(AppError::not_found("OAuth app is inactive"));
        }
        Ok(OAuthCredentials {
            client_id: app.client_id,
            client_secret: app.client_secret,
            redirect_uri: app.redirect_uri,
        })
    }

    pub(super) fn build_authorize_url(
        &self,
        credentials: &OAuthCredentials,
        state: &str,
        code_challenge: &str,
    ) -> AppResult<String> {
        let query = serde_urlencoded::to_string([
            ("client_id", credentials.client_id.as_str()),
            ("redirect_uri", credentials.redirect_uri.as_str()),
            ("response_type", "code"),
            ("code_challenge", code_challenge),
            ("code_challenge_method", "S256"),
            ("state", state),
        ])
        .map_err(|error| AppError::internal(format!("urlencode: {error}")))?;
        Ok(format!("{}/authorize?{query}", self.sc.auth_base_url()))
    }

    pub(super) async fn retry_with_new_app(
        &self,
        login_request: &LoginRequest,
        failed_client_id: &str,
        reason: &str,
        minimum_cooldown: u64,
    ) -> AppResult<()> {
        let app_key = oauth_app_key(failed_client_id);
        let _ = self.health.record_app_failure(&app_key).await;
        let failed_app_id = login_request
            .oauth_app_id
            .as_deref()
            .and_then(|value| Uuid::parse_str(value).ok());
        if let Some(failed_app_id) = failed_app_id
            && let Ok(cooldown_seconds) = self
                .health
                .penalize_app_at_least(failed_app_id, minimum_cooldown)
                .await
        {
            warn!(%app_key, cooldown_seconds, %reason, "OAuth app penalized");
        }

        if login_request.retry_count >= MAX_AUTH_RETRIES {
            self.mark_request_failed(login_request.id, reason).await?;
            return Ok(());
        }

        let (credentials, app_id) = match self.pick_credentials(Some(failed_client_id)).await {
            Ok(value) => value,
            Err(_) => {
                self.mark_request_failed(login_request.id, reason).await?;
                return Ok(());
            }
        };

        let code_verifier = base64_url(&random_bytes(32));
        let code_challenge = base64_url(sha2::Sha256::digest(code_verifier.as_bytes()).as_slice());
        let state = hex::encode(random_bytes(16));
        let url = self.build_authorize_url(&credentials, &state, &code_challenge)?;
        let expires_at =
            (Utc::now() + chrono::Duration::seconds(LOGIN_REQUEST_TTL_SECS)).naive_utc();
        let app_id = app_id.to_string();

        let updated = sqlx::query_file_scalar!(
            "queries/auth/service/retry_login_request.sql",
            login_request.id,
            &state,
            &code_verifier,
            &app_id,
            &url,
            expires_at
        )
        .fetch_optional(&self.pool)
        .await?;
        if updated.is_some() {
            info!(
                attempt = login_request.retry_count + 1,
                "Auth retry prepared"
            );
        }
        Ok(())
    }

    async fn pick_healthy_app(&self, exclude_client_id: Option<&str>) -> AppResult<OAuthApp> {
        let active = self
            .oauth_apps
            .find_all()
            .await?
            .into_iter()
            .filter(|app| {
                app.active
                    && !exclude_client_id
                        .is_some_and(|excluded| same_client_id(excluded, &app.client_id))
            })
            .collect::<Vec<_>>();
        if active.is_empty() {
            return Err(AppError::service_unavailable(
                "No active OAuth apps available",
            ));
        }

        let app_keys = active
            .iter()
            .map(|app| oauth_app_key(&app.client_id))
            .collect::<Vec<_>>();
        let app_identities = active.iter().map(|app| app.id).collect::<Vec<_>>();
        let (health, penalties) = tokio::join!(
            self.health.app_healths_fail_open(&app_keys),
            self.health
                .app_penalties_for_apps_fail_open(&app_identities)
        );
        let available = active
            .iter()
            .filter(|app| !penalties.contains_key(&app.id))
            .collect::<Vec<_>>();
        if available.is_empty() {
            let retry_after = active
                .iter()
                .filter_map(|app| penalties.get(&app.id).copied())
                .min()
                .unwrap_or(1);
            return Err(AppError::soundcloud_refresh_rate_limited(retry_after));
        }

        let preferred = available
            .iter()
            .filter(|app| {
                let app_key = oauth_app_key(&app.client_id);
                health.get(&app_key).is_none_or(|value| !value.unhealthy())
            })
            .map(|app| app.id)
            .collect::<Vec<_>>();
        if !preferred.is_empty() {
            return self.oauth_apps.pick_lru_from(&preferred).await;
        }

        let ids = available.iter().map(|app| app.id).collect::<Vec<_>>();
        self.oauth_apps.pick_lru_from(&ids).await
    }
}

fn credentials_from_app(app: OAuthApp) -> (OAuthCredentials, Uuid) {
    info!(app_name = %app.name, app_id = %app.id, "Auth flow using app");
    let app_id = app.id;
    (
        OAuthCredentials {
            client_id: app.client_id,
            client_secret: app.client_secret,
            redirect_uri: app.redirect_uri,
        },
        app_id,
    )
}

fn same_client_id(left: &str, right: &str) -> bool {
    left.trim() == right.trim()
}

pub(super) fn access_token_subject(token: &str) -> Option<String> {
    let claims = token_claims(token)?;
    let subject = claims.get("sub")?.as_str()?.trim();
    (!subject.is_empty()).then(|| subject.to_owned())
}

pub(super) fn access_token_expiry(token: &str) -> Option<DateTime<Utc>> {
    let timestamp = token_claims(token)?.get("exp")?.as_i64()?;
    DateTime::from_timestamp(timestamp, 0)
}

pub(super) fn public_error_message(error: &AppError, fallback: &str) -> String {
    match error {
        AppError::ScApi { body, .. } => body
            .get("error_description")
            .or_else(|| body.get("message"))
            .and_then(|value| value.as_str())
            .unwrap_or(fallback)
            .to_owned(),
        _ => fallback.to_owned(),
    }
}

pub(super) fn oauth_app_failure_cooldown(error: &AppError) -> Option<u64> {
    if crate::sc::is_ban_error(error) {
        return Some(30 * 60);
    }
    if crate::sc::is_app_credentials_error(error) {
        return Some(30 * 60);
    }
    if crate::sc::is_rate_limited(error) {
        return Some(
            crate::sc::retry_after_seconds(error)
                .and_then(|seconds| u64::try_from(seconds).ok())
                .unwrap_or(5 * 60),
        );
    }
    if crate::sc::is_upstream_failure(error) {
        return Some(30);
    }
    match error {
        AppError::ScUnreachable(_) | AppError::Http(_) => Some(30),
        _ => None,
    }
}

fn token_claims(token: &str) -> Option<serde_json::Value> {
    let payload = token.split('.').nth(1)?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload)
        .ok()?;
    serde_json::from_slice(&bytes).ok()
}
