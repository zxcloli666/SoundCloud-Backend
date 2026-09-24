use std::time::Duration;

use base64::Engine;
use serde::Deserialize;
use sqlx::PgPool;
use uuid::Uuid;

use crate::handlers::oauth_cooldowns::OAuthAppCooldowns;

use super::client::{RefreshedToken, SoundCloudError, TokenRefreshClient};
use super::model::Connection;

const REFRESH_LEASE_SECONDS: i32 = 60;
const REJECTED_TOKEN_RETRY_SECONDS: i32 = 5 * 60;
const TRANSIENT_RETRY_MIN_SECONDS: i32 = 30;
const TRANSIENT_RETRY_MAX_SECONDS: i32 = 15 * 60;
const RATE_LIMIT_RETRY_SECONDS: i32 = 5 * 60;
const APP_CREDENTIALS_RETRY_SECONDS: i64 = 30 * 60;
const BAN_RETRY_SECONDS: i64 = 30 * 60;

#[derive(Clone, Debug)]
pub struct AccessToken {
    pub value: String,
    pub oauth_app_id: Option<Uuid>,
}

#[derive(Debug, thiserror::Error)]
pub enum ConnectionError {
    #[error("SoundCloud connection requires reauthorization")]
    ReauthorizationRequired,

    #[error("SoundCloud connection refresh is already in progress")]
    RefreshInProgress { retry_after_seconds: i64 },

    #[error("SoundCloud connection is rate-limited")]
    RateLimited { retry_after_seconds: i64 },

    #[error("SoundCloud connection is temporarily unavailable")]
    TemporarilyUnavailable { retry_after_seconds: i64 },

    #[error("SoundCloud connection database operation failed: {0}")]
    Database(#[from] sqlx::Error),
}

pub struct ConnectionManager {
    pool: PgPool,
    cooldowns: OAuthAppCooldowns,
}

impl ConnectionManager {
    pub fn new(pool: PgPool) -> Self {
        Self {
            cooldowns: OAuthAppCooldowns::new(pool.clone()),
            pool,
        }
    }

    pub async fn access_token(
        &self,
        client: &TokenRefreshClient,
        user_id: &str,
    ) -> Result<AccessToken, ConnectionError> {
        let connection = self.load(user_id).await?;
        if connection.token_is_usable() {
            return Ok(access_token(&connection));
        }
        self.refresh(client, connection).await
    }

    pub async fn refresh_rejected_token(
        &self,
        client: &TokenRefreshClient,
        user_id: &str,
        rejected_token: &str,
    ) -> Result<AccessToken, ConnectionError> {
        let connection = self.load(user_id).await?;
        if connection.access_token != rejected_token && connection.token_is_usable() {
            return Ok(access_token(&connection));
        }
        self.mark_rejected(&connection, rejected_token, 0).await?;
        self.refresh(client, self.load(user_id).await?).await
    }

    pub async fn penalize_app(
        &self,
        oauth_app_id: Uuid,
        minimum_seconds: i64,
    ) -> Result<i64, ConnectionError> {
        Ok(self
            .cooldowns
            .penalize(oauth_app_id, minimum_seconds)
            .await?)
    }

    pub async fn app_retry_after_seconds(
        &self,
        oauth_app_id: Uuid,
    ) -> Result<Option<i64>, ConnectionError> {
        Ok(self.cooldowns.retry_after_seconds(oauth_app_id).await?)
    }

    pub async fn reject_for_later(
        &self,
        user_id: &str,
        rejected_token: &str,
    ) -> Result<(), ConnectionError> {
        let connection = self.load(user_id).await?;
        self.mark_rejected(&connection, rejected_token, REJECTED_TOKEN_RETRY_SECONDS)
            .await
    }

    async fn load(&self, user_id: &str) -> Result<Connection, ConnectionError> {
        let canonical_user_id = extract_sc_id(user_id);
        sqlx::query_file_as!(
            Connection,
            "queries/sync_queue/connection/load_for_user.sql",
            canonical_user_id
        )
        .fetch_optional(&self.pool)
        .await?
        .ok_or(ConnectionError::ReauthorizationRequired)
    }

    async fn refresh(
        &self,
        client: &TokenRefreshClient,
        connection: Connection,
    ) -> Result<AccessToken, ConnectionError> {
        if connection.last_refresh_error_kind.as_deref() == Some("reauthorization_required") {
            return Err(ConnectionError::ReauthorizationRequired);
        }
        if let Some(retry_after_seconds) = connection.active_refresh_lease_seconds() {
            return Err(ConnectionError::RefreshInProgress {
                retry_after_seconds,
            });
        }
        if let Some(retry_after_seconds) = connection.retry_seconds() {
            if connection.last_refresh_error_kind.as_deref() == Some("rate_limited") {
                return Err(ConnectionError::RateLimited {
                    retry_after_seconds,
                });
            }
            return Err(ConnectionError::TemporarilyUnavailable {
                retry_after_seconds,
            });
        }
        let Some((client_id, client_secret)) = connection.refresh_credentials() else {
            return Err(ConnectionError::ReauthorizationRequired);
        };
        let client_id = client_id.to_owned();
        let client_secret = client_secret.to_owned();
        let oauth_app_id = connection
            .oauth_app_id
            .ok_or(ConnectionError::ReauthorizationRequired)?;
        if let Some(retry_after_seconds) = self.cooldowns.retry_after_seconds(oauth_app_id).await? {
            return Err(ConnectionError::RateLimited {
                retry_after_seconds,
            });
        }
        let lease_id = Uuid::new_v4();
        let refresh_generation = sqlx::query_file_scalar!(
            "queries/sync_queue/connection/claim_refresh.sql",
            connection.id,
            connection.refresh_generation,
            &connection.access_token,
            lease_id,
            REFRESH_LEASE_SECONDS
        )
        .fetch_optional(&self.pool)
        .await?;
        let Some(refresh_generation) = refresh_generation else {
            let current = self.load(&connection.soundcloud_user_id).await?;
            if current.token_is_usable() && current.access_token != connection.access_token {
                return Ok(access_token(&current));
            }
            return Err(ConnectionError::RefreshInProgress {
                retry_after_seconds: current.active_refresh_lease_seconds().unwrap_or(1),
            });
        };

        let refreshed = client
            .refresh(&connection.refresh_token, &client_id, &client_secret)
            .await;
        let token = match refreshed {
            Ok(token) => token,
            Err(error) => {
                return self
                    .record_refresh_failure(
                        &connection,
                        lease_id,
                        refresh_generation,
                        oauth_app_id,
                        error,
                    )
                    .await;
            }
        };
        if token_subject(&token.access_token)
            .is_some_and(|subject| extract_sc_id(&subject) != connection.soundcloud_user_id)
        {
            return self
                .record_refresh_failure(
                    &connection,
                    lease_id,
                    refresh_generation,
                    oauth_app_id,
                    SoundCloudError::InvalidTokenResponse,
                )
                .await;
        }
        let token = self
            .complete_refresh(&connection, lease_id, refresh_generation, token)
            .await?;
        Ok(token)
    }

    async fn complete_refresh(
        &self,
        connection: &Connection,
        lease_id: Uuid,
        refresh_generation: i64,
        token: RefreshedToken,
    ) -> Result<AccessToken, ConnectionError> {
        let scope = token.scope.as_deref().unwrap_or(&connection.scope);
        let refreshed = sqlx::query_file_scalar!(
            "queries/sync_queue/connection/complete_refresh.sql",
            connection.id,
            lease_id,
            refresh_generation,
            &token.access_token,
            &token.refresh_token,
            token.expires_at,
            scope
        )
        .fetch_optional(&self.pool)
        .await?;
        let Some(refreshed) = refreshed else {
            return self.reconcile_refresh_race(connection).await;
        };
        Ok(AccessToken {
            value: refreshed,
            oauth_app_id: connection.oauth_app_id,
        })
    }

    async fn record_refresh_failure(
        &self,
        connection: &Connection,
        lease_id: Uuid,
        refresh_generation: i64,
        oauth_app_id: Uuid,
        error: SoundCloudError,
    ) -> Result<AccessToken, ConnectionError> {
        if error.is_invalid_grant() {
            self.fail_reauthorization(
                connection,
                lease_id,
                refresh_generation,
                "SoundCloud rejected the refresh token",
            )
            .await?;
            return self.reconcile_refresh_race(connection).await;
        }
        let retry_after_seconds = if error.is_rate_limited() {
            error
                .retry_after_seconds()
                .unwrap_or(i64::from(RATE_LIMIT_RETRY_SECONDS))
                .clamp(1, i64::from(i32::MAX)) as i32
        } else {
            refresh_backoff(connection.refresh_failure_count)
        };
        let kind = if error.is_rate_limited() {
            "rate_limited"
        } else if matches!(&error, SoundCloudError::Transport(source) if source.is_timeout()) {
            "timed_out"
        } else {
            "temporarily_unavailable"
        };
        let app_wide_retry = app_wide_retry_seconds(&error, i64::from(retry_after_seconds));
        let retry_after_seconds = match app_wide_retry {
            Some(minimum) => self
                .cooldowns
                .penalize(oauth_app_id, minimum)
                .await?
                .clamp(1, i64::from(i32::MAX)) as i32,
            None => retry_after_seconds,
        };
        let message = bounded_message(&error.to_string());
        let recorded = sqlx::query_file!(
            "queries/sync_queue/connection/fail_retryable.sql",
            connection.id,
            lease_id,
            refresh_generation,
            kind,
            message,
            retry_after_seconds
        )
        .execute(&self.pool)
        .await?;
        if recorded.rows_affected() == 0 {
            return self.reconcile_refresh_race(connection).await;
        }
        if kind == "rate_limited" {
            Err(ConnectionError::RateLimited {
                retry_after_seconds: i64::from(retry_after_seconds),
            })
        } else {
            Err(ConnectionError::TemporarilyUnavailable {
                retry_after_seconds: i64::from(retry_after_seconds),
            })
        }
    }

    async fn fail_reauthorization(
        &self,
        connection: &Connection,
        lease_id: Uuid,
        refresh_generation: i64,
        message: &str,
    ) -> Result<(), ConnectionError> {
        sqlx::query_file!(
            "../api/queries/auth/service/fail_connection_refresh_reauth.sql",
            connection.id,
            lease_id,
            refresh_generation,
            bounded_message(message)
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn reconcile_refresh_race(
        &self,
        previous: &Connection,
    ) -> Result<AccessToken, ConnectionError> {
        let current = self.load(&previous.soundcloud_user_id).await?;
        if current.token_is_usable()
            && (current.refresh_generation != previous.refresh_generation
                || current.access_token != previous.access_token)
        {
            return Ok(access_token(&current));
        }
        if current.last_refresh_error_kind.as_deref() == Some("reauthorization_required") {
            return Err(ConnectionError::ReauthorizationRequired);
        }
        if let Some(retry_after_seconds) = current.active_refresh_lease_seconds() {
            return Err(ConnectionError::RefreshInProgress {
                retry_after_seconds,
            });
        }
        if let Some(retry_after_seconds) = current.retry_seconds() {
            if current.last_refresh_error_kind.as_deref() == Some("rate_limited") {
                return Err(ConnectionError::RateLimited {
                    retry_after_seconds,
                });
            }
            return Err(ConnectionError::TemporarilyUnavailable {
                retry_after_seconds,
            });
        }
        Err(ConnectionError::TemporarilyUnavailable {
            retry_after_seconds: 1,
        })
    }

    async fn mark_rejected(
        &self,
        connection: &Connection,
        rejected_token: &str,
        retry_seconds: i32,
    ) -> Result<(), ConnectionError> {
        sqlx::query_file!(
            "queries/sync_queue/connection/mark_rejected.sql",
            connection.id,
            rejected_token,
            retry_seconds
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }
}

fn app_wide_retry_seconds(error: &SoundCloudError, transient_seconds: i64) -> Option<i64> {
    if error.is_rate_limited() {
        return Some(
            error
                .retry_after_seconds()
                .unwrap_or(RATE_LIMIT_RETRY_SECONDS.into()),
        );
    }
    if error.is_app_credentials_error() {
        return Some(APP_CREDENTIALS_RETRY_SECONDS);
    }
    if error.is_banned() {
        return Some(BAN_RETRY_SECONDS);
    }
    match error {
        SoundCloudError::Api { status, .. } if status.is_server_error() => Some(transient_seconds),
        SoundCloudError::Transport(_) | SoundCloudError::ResponseTooLarge => {
            Some(transient_seconds)
        }
        SoundCloudError::InvalidTokenResponse => None,
        SoundCloudError::Api { .. } => None,
    }
}

fn access_token(connection: &Connection) -> AccessToken {
    AccessToken {
        value: connection.access_token.clone(),
        oauth_app_id: connection.oauth_app_id,
    }
}

fn refresh_backoff(failure_count: i32) -> i32 {
    let exponent = u32::try_from(failure_count.max(0)).map_or(0, |value| value.min(5));
    let seconds = Duration::from_secs(TRANSIENT_RETRY_MIN_SECONDS as u64)
        .saturating_mul(2_u32.saturating_pow(exponent))
        .min(Duration::from_secs(TRANSIENT_RETRY_MAX_SECONDS as u64))
        .as_secs();
    i32::try_from(seconds).unwrap_or(TRANSIENT_RETRY_MAX_SECONDS)
}

fn extract_sc_id(value: &str) -> &str {
    value.rsplit(':').next().unwrap_or(value)
}

#[derive(Deserialize)]
struct TokenClaims {
    sub: Option<String>,
}

fn token_subject(token: &str) -> Option<String> {
    let payload = token.split('.').nth(1)?;
    let decoded = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload)
        .ok()?;
    serde_json::from_slice::<TokenClaims>(&decoded).ok()?.sub
}

fn bounded_message(message: &str) -> String {
    message.chars().take(500).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refresh_backoff_grows_and_stays_bounded() {
        assert!(refresh_backoff(1) > refresh_backoff(0));
        assert_eq!(refresh_backoff(i32::MAX), TRANSIENT_RETRY_MAX_SECONDS);
    }

    #[test]
    fn soundcloud_urn_is_reduced_to_its_entity_id() {
        assert_eq!(extract_sc_id("soundcloud:users:42"), "42");
    }

    #[test]
    fn app_wide_failures_share_one_cooldown() {
        let server_error = SoundCloudError::Api {
            status: wreq::StatusCode::SERVICE_UNAVAILABLE,
            body: serde_json::Value::Null,
            retry_after_seconds: None,
        };
        let invalid_grant = SoundCloudError::Api {
            status: wreq::StatusCode::BAD_REQUEST,
            body: serde_json::json!({ "error": "invalid_grant" }),
            retry_after_seconds: None,
        };

        assert_eq!(app_wide_retry_seconds(&server_error, 30), Some(30));
        assert_eq!(app_wide_retry_seconds(&invalid_grant, 30), None);
    }
}
