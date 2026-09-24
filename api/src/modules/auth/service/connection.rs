use std::time::Duration;

use chrono::Utc;
use tracing::{info, warn};
use uuid::Uuid;

use crate::error::{AppError, AppResult};
use crate::modules::auth::dto::{SoundCloudConnectionResponse, SoundCloudConnectionState};
use crate::modules::auth::health::oauth_app_key;
use crate::modules::auth::model::{AuthSession, SoundCloudConnection, SoundCloudConnectionStatus};
use crate::sc;

use super::AuthService;
use super::oauth::{
    access_token_expiry, access_token_subject, oauth_app_failure_cooldown, public_error_message,
};

const REFRESH_DUE_BUFFER: Duration = Duration::from_secs(5 * 60);
const REFRESH_TIMEOUT: Duration = Duration::from_secs(10);
const REFRESH_LEASE_SECONDS: i32 = 60;
const REJECTED_TOKEN_RETRY_SECONDS: i32 = 5 * 60;
const RETRY_TRANSIENT_BASE_SECONDS: i32 = 30;
const RETRY_TRANSIENT_MAX_SECONDS: i32 = 15 * 60;
const RETRY_RATE_LIMIT_BASE_SECONDS: i32 = 5 * 60;
const RETRY_RATE_LIMIT_MAX_SECONDS: i32 = 30 * 60;

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
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

pub struct RefreshAttempt {
    pub outcome: RefreshOutcome,
    pub connection: Option<SoundCloudConnection>,
}

impl RefreshAttempt {
    pub fn response(&self) -> SoundCloudConnectionResponse {
        let mut response = connection_response(self.connection.as_ref());
        match self.outcome {
            RefreshOutcome::ReauthorizationRequired => {
                response.state = SoundCloudConnectionState::ReauthorizationRequired;
                response.can_use_soundcloud = false;
                response.can_refresh = false;
                response.error_code = Some("reauthorization_required".to_owned());
                response.error_message =
                    Some("SoundCloud repeatedly rejected the refresh token".to_owned());
            }
            RefreshOutcome::NotConnected => {
                response.state = SoundCloudConnectionState::NotConnected;
                response.can_use_soundcloud = false;
                response.can_refresh = false;
            }
            _ => {}
        }
        response
    }
}

impl AuthService {
    pub async fn get_valid_access_token(&self, session_id: Uuid) -> AppResult<String> {
        let connection = self
            .get_connection_for_session(session_id)
            .await?
            .ok_or_else(AppError::soundcloud_reauthorization_required)?;
        self.valid_access_token(connection).await
    }

    pub async fn get_immediately_usable_access_token(&self, session_id: Uuid) -> AppResult<String> {
        let connection = self
            .get_connection_for_session(session_id)
            .await?
            .ok_or_else(AppError::soundcloud_reauthorization_required)?;
        access_token_is_usable(&connection)
            .then_some(connection.access_token)
            .ok_or_else(AppError::soundcloud_temporarily_unavailable)
    }

    async fn valid_access_token(&self, connection: SoundCloudConnection) -> AppResult<String> {
        if access_token_is_usable(&connection) {
            return Ok(connection.access_token);
        }

        let attempt = self.refresh_connection_once(connection).await?;
        if let Some(connection) = attempt.connection.as_ref()
            && access_token_is_usable(connection)
            && !matches!(attempt.outcome, RefreshOutcome::NotConnected)
        {
            return Ok(connection.access_token.clone());
        }

        let retry_after = attempt.response().retry_after_sec;
        match attempt.outcome {
            RefreshOutcome::ReauthorizationRequired | RefreshOutcome::NotConnected => {
                Err(AppError::soundcloud_reauthorization_required())
            }
            RefreshOutcome::RateLimited => Err(AppError::soundcloud_refresh_rate_limited(
                retry_after.unwrap_or(1),
            )),
            RefreshOutcome::TimedOut => Err(AppError::soundcloud_refresh_timed_out()),
            _ => Err(AppError::soundcloud_temporarily_unavailable_for(
                retry_after,
            )),
        }
    }

    pub async fn soundcloud_status(
        &self,
        session_id: Uuid,
    ) -> AppResult<SoundCloudConnectionResponse> {
        let session = self
            .get_auth_session(session_id)
            .await?
            .ok_or_else(|| AppError::unauthorized("Session not found"))?;
        Ok(auth_session_response(&session))
    }

    pub async fn refresh_soundcloud(&self, session_id: Uuid) -> AppResult<RefreshAttempt> {
        let session = self
            .get_auth_session(session_id)
            .await?
            .ok_or_else(|| AppError::unauthorized("Session not found"))?;
        let Some(connection) = self.get_connection_for_session(session_id).await? else {
            return Ok(RefreshAttempt {
                outcome: RefreshOutcome::NotConnected,
                connection: None,
            });
        };
        if session.oauth_app_id.is_none() || session.oauth_app_active != Some(true) {
            return Err(AppError::soundcloud_temporarily_unavailable_for(Some(1800)));
        }
        if !refresh_due(&connection) {
            return Ok(attempt(RefreshOutcome::AlreadyFresh, connection));
        }
        self.refresh_connection_once(connection).await
    }

    pub async fn refresh_rejected_access_token(
        &self,
        session_id: Uuid,
        rejected_access_token: &str,
    ) -> AppResult<String> {
        let connection = self
            .get_connection_for_session(session_id)
            .await?
            .ok_or_else(AppError::soundcloud_reauthorization_required)?;
        if connection.access_token != rejected_access_token && access_token_is_usable(&connection) {
            return Ok(connection.access_token);
        }
        let attempt = self.refresh_connection_once(connection).await?;
        let retry_after = attempt.response().retry_after_sec;
        match attempt.outcome {
            RefreshOutcome::Refreshed => attempt
                .connection
                .filter(access_token_is_usable)
                .map(|connection| connection.access_token)
                .ok_or_else(AppError::soundcloud_reauthorization_required),
            RefreshOutcome::AlreadyFresh => attempt
                .connection
                .filter(|connection| {
                    connection.access_token != rejected_access_token
                        && access_token_is_usable(connection)
                })
                .map(|connection| connection.access_token)
                .ok_or_else(AppError::soundcloud_temporarily_unavailable),
            RefreshOutcome::ReauthorizationRequired | RefreshOutcome::NotConnected => {
                Err(AppError::soundcloud_reauthorization_required())
            }
            RefreshOutcome::RateLimited => Err(AppError::soundcloud_refresh_rate_limited(
                retry_after.unwrap_or(1),
            )),
            RefreshOutcome::TimedOut => Err(AppError::soundcloud_refresh_timed_out()),
            RefreshOutcome::InProgress | RefreshOutcome::RetryLater => Err(
                AppError::soundcloud_temporarily_unavailable_for(retry_after),
            ),
        }
    }

    pub async fn mark_access_token_rejected(
        &self,
        session_id: Uuid,
        rejected_access_token: &str,
    ) -> AppResult<Option<i64>> {
        let retry_after = sqlx::query_file_scalar!(
            "queries/auth/service/mark_connection_token_rejected.sql",
            session_id,
            rejected_access_token,
            REJECTED_TOKEN_RETRY_SECONDS
        )
        .fetch_optional(&self.pool)
        .await?;
        Ok(retry_after)
    }

    async fn refresh_connection_once(
        &self,
        connection: SoundCloudConnection,
    ) -> AppResult<RefreshAttempt> {
        if connection.last_refresh_error_kind.as_deref() == Some("reauthorization_required") {
            return Ok(attempt(RefreshOutcome::ReauthorizationRequired, connection));
        }
        if lease_active(&connection) {
            return Ok(attempt(RefreshOutcome::InProgress, connection));
        }
        if retry_pending(&connection) {
            return Ok(attempt(retry_outcome(&connection), connection));
        }

        let lease_id = Uuid::new_v4();
        let claimed = sqlx::query_file_as!(
            SoundCloudConnection,
            "queries/auth/service/claim_connection_refresh.sql",
            connection.id,
            connection.refresh_generation,
            connection.access_token,
            lease_id,
            REFRESH_LEASE_SECONDS
        )
        .fetch_optional(&self.pool)
        .await?;

        let Some(claimed) = claimed else {
            return self.refresh_race_outcome(connection.id).await;
        };

        let credentials = match self.credentials_for_connection(&claimed).await {
            Ok(credentials) => credentials,
            Err(error) => {
                warn!(connection = %claimed.id, %error, "OAuth credentials lookup failed");
                let retry_seconds = retry_delay(
                    &claimed,
                    RETRY_TRANSIENT_BASE_SECONDS,
                    RETRY_TRANSIENT_MAX_SECONDS,
                );
                self.record_retryable_failure(
                    &claimed,
                    lease_id,
                    "temporarily_unavailable",
                    "OAuth credentials are temporarily unavailable",
                    retry_seconds,
                )
                .await?;
                return self.refresh_race_outcome(claimed.id).await;
            }
        };
        let app_key = oauth_app_key(&credentials.client_id);
        let oauth_app_id = claimed
            .oauth_app_id
            .ok_or_else(|| AppError::internal("connection OAuth app is missing"))?;

        let refreshed = tokio::time::timeout(
            REFRESH_TIMEOUT,
            self.health.run_oauth_request(
                oauth_app_id,
                self.sc
                    .refresh_access_token(&claimed.refresh_token, &credentials),
            ),
        )
        .await;

        let token = match refreshed {
            Ok(Ok(token)) => token,
            Ok(Err(error)) => {
                return self
                    .record_refresh_error(claimed, lease_id, oauth_app_id, &app_key, error)
                    .await;
            }
            Err(_) => {
                let retry_seconds = self
                    .app_failure_delay(
                        &claimed,
                        oauth_app_id,
                        &app_key,
                        RETRY_TRANSIENT_BASE_SECONDS,
                        RETRY_TRANSIENT_MAX_SECONDS,
                        0,
                    )
                    .await;
                self.record_retryable_failure(
                    &claimed,
                    lease_id,
                    "timed_out",
                    "SoundCloud token refresh timed out",
                    retry_seconds,
                )
                .await?;
                return self.refresh_race_outcome(claimed.id).await;
            }
        };

        if !refreshed_token_is_complete(&token) {
            self.record_retryable_failure(
                &claimed,
                lease_id,
                "temporarily_unavailable",
                "SoundCloud returned an incomplete token response",
                RETRY_TRANSIENT_MAX_SECONDS,
            )
            .await?;
            return self.refresh_race_outcome(claimed.id).await;
        }

        if let Some(subject) = access_token_subject(&token.access_token)
            && crate::common::sc_ids::extract_sc_id(&subject) != claimed.soundcloud_user_id
        {
            self.record_retryable_failure(
                &claimed,
                lease_id,
                "temporarily_unavailable",
                "SoundCloud returned credentials for another account",
                RETRY_TRANSIENT_MAX_SECONDS,
            )
            .await?;
            return self.refresh_race_outcome(claimed.id).await;
        }

        let expires_at = access_token_expiry(&token.access_token)
            .unwrap_or_else(|| Utc::now() + chrono::Duration::seconds(token.expires_in.max(1)));
        let scope = if token.scope.is_empty() {
            claimed.scope.as_str()
        } else {
            token.scope.as_str()
        };
        let updated = sqlx::query_file_as!(
            SoundCloudConnection,
            "queries/auth/service/complete_connection_refresh.sql",
            claimed.id,
            lease_id,
            claimed.refresh_generation,
            token.access_token,
            token.refresh_token,
            expires_at,
            scope
        )
        .fetch_optional(&self.pool)
        .await?;

        let Some(updated) = updated else {
            return self.refresh_race_outcome(claimed.id).await;
        };

        let _ = self.health.record_app_success(&app_key).await;
        info!(connection = %updated.id, "SoundCloud connection refreshed");
        Ok(attempt(RefreshOutcome::Refreshed, updated))
    }

    async fn record_refresh_error(
        &self,
        connection: SoundCloudConnection,
        lease_id: Uuid,
        oauth_app_id: Uuid,
        app_key: &str,
        error: AppError,
    ) -> AppResult<RefreshAttempt> {
        let message = public_error_message(&error, "SoundCloud token refresh failed");
        let outcome = if let AppError::SoundCloudRefreshRateLimited { retry_after_sec } = &error {
            let retry_seconds = i32::try_from((*retry_after_sec).max(1)).unwrap_or(i32::MAX);
            self.record_retryable_failure(
                &connection,
                lease_id,
                "rate_limited",
                &message,
                retry_seconds,
            )
            .await?;
            RefreshOutcome::RateLimited
        } else if sc::is_rate_limited(&error) {
            let upstream_retry = sc::retry_after_seconds(&error).unwrap_or_default();
            let retry_seconds = self
                .app_failure_delay(
                    &connection,
                    oauth_app_id,
                    app_key,
                    RETRY_RATE_LIMIT_BASE_SECONDS,
                    RETRY_RATE_LIMIT_MAX_SECONDS,
                    upstream_retry,
                )
                .await;
            self.record_retryable_failure(
                &connection,
                lease_id,
                "rate_limited",
                &message,
                retry_seconds,
            )
            .await?;
            RefreshOutcome::RateLimited
        } else if sc::is_invalid_grant(&error) {
            self.record_reauthorization_required(&connection, lease_id, &message)
                .await?;
            RefreshOutcome::ReauthorizationRequired
        } else if sc::is_app_credentials_error(&error) {
            let retry_seconds = self
                .app_failure_delay(
                    &connection,
                    oauth_app_id,
                    app_key,
                    RETRY_TRANSIENT_BASE_SECONDS,
                    RETRY_TRANSIENT_MAX_SECONDS,
                    30 * 60,
                )
                .await;
            self.record_retryable_failure(
                &connection,
                lease_id,
                "temporarily_unavailable",
                &message,
                retry_seconds,
            )
            .await?;
            RefreshOutcome::RetryLater
        } else if let Some(minimum_cooldown) = oauth_app_failure_cooldown(&error) {
            crate::metrics::record_sc_failure(&error);
            let minimum_cooldown = i64::try_from(minimum_cooldown).unwrap_or(i64::MAX);
            let retry_seconds = self
                .app_failure_delay(
                    &connection,
                    oauth_app_id,
                    app_key,
                    RETRY_TRANSIENT_BASE_SECONDS,
                    RETRY_TRANSIENT_MAX_SECONDS,
                    minimum_cooldown,
                )
                .await;
            self.record_retryable_failure(
                &connection,
                lease_id,
                "temporarily_unavailable",
                &message,
                retry_seconds,
            )
            .await?;
            RefreshOutcome::RetryLater
        } else {
            let retry_seconds = retry_delay(
                &connection,
                RETRY_TRANSIENT_BASE_SECONDS,
                RETRY_TRANSIENT_MAX_SECONDS,
            );
            self.record_retryable_failure(
                &connection,
                lease_id,
                "temporarily_unavailable",
                &message,
                retry_seconds,
            )
            .await?;
            RefreshOutcome::RetryLater
        };
        warn!(connection = %connection.id, %error, ?outcome, "SoundCloud refresh failed");
        self.refresh_race_outcome(connection.id).await
    }

    async fn app_failure_delay(
        &self,
        connection: &SoundCloudConnection,
        oauth_app_id: Uuid,
        app_key: &str,
        base_seconds: i32,
        max_seconds: i32,
        minimum_seconds: i64,
    ) -> i32 {
        let connection_delay = retry_delay(connection, base_seconds, max_seconds);
        let minimum_seconds =
            i32::try_from(minimum_seconds.clamp(0, i64::from(i32::MAX))).unwrap_or(i32::MAX);
        let minimum_seconds = connection_delay.max(minimum_seconds);
        let _ = self.health.record_app_failure(app_key).await;
        match self
            .health
            .penalize_app_at_least(oauth_app_id, minimum_seconds as u64)
            .await
        {
            Ok(seconds) => i32::try_from(seconds)
                .unwrap_or(i32::MAX)
                .max(minimum_seconds),
            Err(error) => {
                warn!(%app_key, %error, "OAuth app cooldown update failed");
                minimum_seconds
            }
        }
    }

    async fn record_retryable_failure(
        &self,
        connection: &SoundCloudConnection,
        lease_id: Uuid,
        kind: &str,
        message: &str,
        retry_seconds: i32,
    ) -> AppResult<()> {
        let message = bounded_message(message);
        sqlx::query_file!(
            "queries/auth/service/fail_connection_refresh_retryable.sql",
            connection.id,
            lease_id,
            connection.refresh_generation,
            kind,
            message,
            retry_seconds
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn record_reauthorization_required(
        &self,
        connection: &SoundCloudConnection,
        lease_id: Uuid,
        message: &str,
    ) -> AppResult<()> {
        let message = bounded_message(message);
        sqlx::query_file!(
            "queries/auth/service/fail_connection_refresh_reauth.sql",
            connection.id,
            lease_id,
            connection.refresh_generation,
            message
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn refresh_race_outcome(&self, connection_id: Uuid) -> AppResult<RefreshAttempt> {
        let Some(connection) = self.get_connection_by_id(connection_id).await? else {
            return Ok(RefreshAttempt {
                outcome: RefreshOutcome::NotConnected,
                connection: None,
            });
        };
        let outcome = if connection.expires_at > Utc::now()
            && connection.refresh_lease_id.is_none()
            && connection.last_refresh_error_kind.is_none()
        {
            RefreshOutcome::AlreadyFresh
        } else if connection.last_refresh_error_kind.as_deref() == Some("reauthorization_required")
        {
            RefreshOutcome::ReauthorizationRequired
        } else if lease_active(&connection) {
            RefreshOutcome::InProgress
        } else if retry_pending(&connection) {
            retry_outcome(&connection)
        } else {
            RefreshOutcome::RetryLater
        };
        Ok(attempt(outcome, connection))
    }
}

pub fn auth_session_response(session: &AuthSession) -> SoundCloudConnectionResponse {
    let status = session.connection_status();
    connection_status_response(status.as_ref())
}

fn connection_response(connection: Option<&SoundCloudConnection>) -> SoundCloudConnectionResponse {
    let status = connection.map(SoundCloudConnectionStatus::from);
    connection_status_response(status.as_ref())
}

fn connection_status_response(
    connection: Option<&SoundCloudConnectionStatus>,
) -> SoundCloudConnectionResponse {
    let Some(connection) = connection else {
        return SoundCloudConnectionResponse {
            state: SoundCloudConnectionState::NotConnected,
            can_use_soundcloud: false,
            can_refresh: false,
            soundcloud_user_id: None,
            soundcloud_urn: None,
            access_token_expires_at: None,
            expires_in_sec: None,
            last_attempt_at: None,
            last_success_at: None,
            retry_after_sec: None,
            error_code: None,
            error_message: None,
        };
    };

    let now = Utc::now();
    let expires_in = connection
        .expires_at
        .signed_duration_since(now)
        .num_seconds();
    let lease_is_active = refresh_lease_active(
        connection.refresh_lease_id,
        connection.refresh_lease_expires_at,
    );
    let retry_is_pending = refresh_retry_pending(connection.retry_at);
    let token_is_rejected = connection.last_refresh_error_kind.as_deref() == Some("token_rejected");
    let refresh_is_due = expires_in <= REFRESH_DUE_BUFFER.as_secs() as i64 || token_is_rejected;
    let source_is_available = connection.has_refresh_credentials;
    let retry_after = if lease_is_active {
        connection.refresh_lease_expires_at
    } else if retry_is_pending {
        connection.retry_at
    } else {
        None
    }
    .map(|retry_at| seconds_until(retry_at, now));
    let state = if connection.last_refresh_error_kind.as_deref() == Some("reauthorization_required")
    {
        SoundCloudConnectionState::ReauthorizationRequired
    } else if !source_is_available {
        SoundCloudConnectionState::RetryLater
    } else if lease_is_active {
        SoundCloudConnectionState::Refreshing
    } else if retry_is_pending {
        SoundCloudConnectionState::RetryLater
    } else if refresh_is_due {
        SoundCloudConnectionState::RefreshDue
    } else {
        SoundCloudConnectionState::Ready
    };

    let active_error = match &state {
        SoundCloudConnectionState::RetryLater if !source_is_available => Some((
            "oauth_app_unavailable".to_owned(),
            "The SoundCloud connection configuration is temporarily unavailable".to_owned(),
        )),
        SoundCloudConnectionState::ReauthorizationRequired
        | SoundCloudConnectionState::RetryLater => Some((
            connection
                .last_refresh_error_kind
                .clone()
                .unwrap_or_else(|| "temporarily_unavailable".to_owned()),
            connection
                .last_refresh_error
                .clone()
                .unwrap_or_else(|| "SoundCloud connection is temporarily unavailable".to_owned()),
        )),
        _ => None,
    };

    SoundCloudConnectionResponse {
        state,
        can_use_soundcloud: expires_in > 0
            && !token_is_rejected
            && connection.last_refresh_error_kind.as_deref() != Some("reauthorization_required"),
        can_refresh: source_is_available
            && refresh_is_due
            && !lease_is_active
            && !retry_is_pending
            && connection.last_refresh_error_kind.as_deref() != Some("reauthorization_required"),
        soundcloud_user_id: Some(connection.soundcloud_user_id.clone()),
        soundcloud_urn: Some(crate::common::sc_ids::user_urn(
            &connection.soundcloud_user_id,
        )),
        access_token_expires_at: Some(connection.expires_at),
        expires_in_sec: Some(expires_in),
        last_attempt_at: connection.last_refresh_attempt_at,
        last_success_at: connection.last_refresh_success_at,
        retry_after_sec: retry_after.or_else(|| (!source_is_available).then_some(1800)),
        error_code: active_error.as_ref().map(|(code, _)| code.clone()),
        error_message: active_error.map(|(_, message)| message),
    }
}

fn attempt(outcome: RefreshOutcome, connection: SoundCloudConnection) -> RefreshAttempt {
    RefreshAttempt {
        outcome,
        connection: Some(connection),
    }
}

fn lease_active(connection: &SoundCloudConnection) -> bool {
    refresh_lease_active(
        connection.refresh_lease_id,
        connection.refresh_lease_expires_at,
    )
}

fn retry_pending(connection: &SoundCloudConnection) -> bool {
    refresh_retry_pending(connection.retry_at)
}

fn refresh_lease_active(
    lease_id: Option<Uuid>,
    lease_expires_at: Option<chrono::DateTime<Utc>>,
) -> bool {
    lease_id.is_some() && lease_expires_at.is_some_and(|expires_at| expires_at > Utc::now())
}

fn refresh_retry_pending(retry_at: Option<chrono::DateTime<Utc>>) -> bool {
    retry_at.is_some_and(|retry_at| retry_at > Utc::now())
}

fn seconds_until(retry_at: chrono::DateTime<Utc>, now: chrono::DateTime<Utc>) -> i64 {
    retry_at
        .signed_duration_since(now)
        .num_milliseconds()
        .max(1)
        .saturating_add(999)
        / 1_000
}

fn retry_outcome(connection: &SoundCloudConnection) -> RefreshOutcome {
    match connection.last_refresh_error_kind.as_deref() {
        Some("rate_limited") => RefreshOutcome::RateLimited,
        Some("timed_out") => RefreshOutcome::TimedOut,
        _ => RefreshOutcome::RetryLater,
    }
}

fn refresh_due(connection: &SoundCloudConnection) -> bool {
    connection.last_refresh_error_kind.as_deref() == Some("token_rejected")
        || connection.expires_at
            <= Utc::now()
                + chrono::Duration::from_std(REFRESH_DUE_BUFFER)
                    .expect("refresh due buffer must fit chrono duration")
}

fn access_token_is_usable(connection: &SoundCloudConnection) -> bool {
    connection.expires_at > Utc::now()
        && !matches!(
            connection.last_refresh_error_kind.as_deref(),
            Some("token_rejected" | "reauthorization_required")
        )
}

fn retry_delay(connection: &SoundCloudConnection, base_seconds: i32, max_seconds: i32) -> i32 {
    let exponent = u32::try_from(connection.refresh_failure_count)
        .unwrap_or_default()
        .min(10);
    let floor = base_seconds
        .saturating_mul(1_i32.checked_shl(exponent).unwrap_or(i32::MAX))
        .min(max_seconds);
    if floor >= max_seconds {
        return max_seconds;
    }
    let width = max_seconds.min(floor.saturating_mul(2)) - floor;
    let salt =
        connection.id.as_u128() ^ (u128::from(exponent) + 1).wrapping_mul(0x9e37_79b9_7f4a_7c15);
    floor + i32::try_from(salt % (u128::from(width as u32) + 1)).unwrap_or_default()
}

fn bounded_message(message: &str) -> String {
    message.chars().take(500).collect()
}

fn refreshed_token_is_complete(token: &sc::ScTokenResponse) -> bool {
    !token.access_token.is_empty() && !token.refresh_token.is_empty()
}

#[cfg(test)]
mod tests;
