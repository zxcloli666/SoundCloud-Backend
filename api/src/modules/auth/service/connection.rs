use std::time::Duration;

use backend_contracts::auth::{ConnectionState, RefreshReply, RefreshRequest, token_fingerprint};
use chrono::{DateTime, Utc};
use tokio::time::Instant;
use uuid::Uuid;

use crate::error::{AppError, AppResult};
use crate::modules::auth::dto::{SoundCloudConnectionResponse, SoundCloudConnectionState};
use crate::modules::auth::model::{AuthSession, SoundCloudConnection, SoundCloudConnectionStatus};

use super::AuthService;

pub use backend_contracts::auth::RefreshOutcome;

const REFRESH_DUE_BUFFER: Duration = Duration::from_secs(5 * 60);
const LEASE_WAIT: Duration = Duration::from_secs(12);
const LEASE_POLL: Duration = Duration::from_millis(500);
const OAUTH_APP_RETRY_SECONDS: i64 = 30 * 60;
const OAUTH_APP_GONE: &str =
    "The SoundCloud app of this login is no longer available, sign in again";

pub struct RefreshAttempt {
    pub outcome: RefreshOutcome,
    pub connection: Option<SoundCloudConnection>,
    pub retry_after_sec: Option<i64>,
}

impl RefreshAttempt {
    pub fn response(&self) -> SoundCloudConnectionResponse {
        let mut response = connection_response(self.connection.as_ref());
        match self.outcome {
            RefreshOutcome::ReauthorizationRequired => {
                let app_is_gone = self.connection.as_ref().is_some_and(|connection| {
                    connection.last_refresh_error_kind.as_deref()
                        != Some("reauthorization_required")
                });
                let (code, message) = if app_is_gone {
                    ("oauth_app_unavailable", OAUTH_APP_GONE)
                } else {
                    (
                        "reauthorization_required",
                        "SoundCloud repeatedly rejected the refresh token",
                    )
                };
                response.state = SoundCloudConnectionState::ReauthorizationRequired;
                response.can_use_soundcloud = false;
                response.can_refresh = false;
                response.retry_after_sec = None;
                response.error_code = Some(code.to_owned());
                response.error_message = Some(message.to_owned());
            }
            RefreshOutcome::NotConnected => {
                response.state = SoundCloudConnectionState::NotConnected;
                response.can_use_soundcloud = false;
                response.can_refresh = false;
            }
            _ => response.retry_after_sec = response.retry_after_sec.or(self.retry_after_sec),
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

    pub async fn get_lasting_access_token(&self, session_id: Uuid) -> AppResult<String> {
        let connection = self
            .get_connection_for_session(session_id)
            .await?
            .ok_or_else(AppError::soundcloud_reauthorization_required)?;
        if !refresh_due(&connection) || !access_token_is_usable(&connection) {
            return self.valid_access_token(connection).await;
        }
        match self.refresh_connection(connection.clone(), None).await {
            Ok(attempt) if attempt.outcome == RefreshOutcome::ReauthorizationRequired => {
                Err(AppError::soundcloud_reauthorization_required())
            }
            Ok(RefreshAttempt {
                connection: Some(refreshed),
                ..
            }) if access_token_is_usable(&refreshed) => Ok(refreshed.access_token),
            _ => self.valid_access_token(connection).await,
        }
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

        let connection_id = connection.id;
        let wait_ends = Instant::now() + LEASE_WAIT;
        let mut attempt = self.refresh_connection(connection, None).await?;
        while attempt.outcome == RefreshOutcome::InProgress && Instant::now() < wait_ends {
            tokio::time::sleep(LEASE_POLL).await;
            let connection = self
                .get_connection_by_id(connection_id)
                .await?
                .ok_or_else(AppError::soundcloud_reauthorization_required)?;
            if access_token_is_usable(&connection) {
                return Ok(connection.access_token);
            }
            attempt = self.refresh_connection(connection, None).await?;
        }

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
            .find_auth_session(session_id)
            .await?
            .ok_or_else(|| AppError::unauthorized("Session not found"))?;
        Ok(auth_session_response(&session))
    }

    pub async fn refresh_soundcloud(&self, session_id: Uuid) -> AppResult<RefreshAttempt> {
        self.find_auth_session(session_id)
            .await?
            .ok_or_else(|| AppError::unauthorized("Session not found"))?;
        let Some(connection) = self.get_connection_for_session(session_id).await? else {
            return Ok(RefreshAttempt {
                outcome: RefreshOutcome::NotConnected,
                connection: None,
                retry_after_sec: None,
            });
        };
        if !refresh_due(&connection) {
            return Ok(attempt(RefreshOutcome::AlreadyFresh, connection));
        }
        self.refresh_connection(connection, None).await
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
        let attempt = self
            .refresh_connection(connection, Some(rejected_access_token))
            .await?;
        let retry_after = attempt.response().retry_after_sec;
        match attempt.outcome {
            RefreshOutcome::Refreshed | RefreshOutcome::AlreadyFresh => attempt
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
        let Some(connection) = self.get_connection_for_session(session_id).await? else {
            return Ok(None);
        };
        let held = self
            .core
            .refresh(&RefreshRequest {
                connection_id: connection.id,
                rejected_token_fingerprint: Some(token_fingerprint(rejected_access_token)),
                hold: true,
            })
            .await;
        let Ok(RefreshReply {
            connection: Some(state),
            ..
        }) = held
        else {
            return Ok(None);
        };
        let now = Utc::now();
        Ok((state.access_token == rejected_access_token
            && state.last_refresh_error_kind.as_deref() == Some("token_rejected"))
        .then(|| state.retry_at_unix_ms.and_then(DateTime::from_timestamp_millis))
        .flatten()
        .filter(|retry_at| *retry_at > now)
        .map(|retry_at| seconds_until(retry_at, now)))
    }

    async fn refresh_connection(
        &self,
        connection: SoundCloudConnection,
        rejected_access_token: Option<&str>,
    ) -> AppResult<RefreshAttempt> {
        if connection.last_refresh_error_kind.as_deref() == Some("reauthorization_required") {
            return Ok(attempt(RefreshOutcome::ReauthorizationRequired, connection));
        }
        match self.credentials_for_connection(&connection).await {
            Ok(_) => {}
            Err(AppError::NotFound(_)) => {
                if self.oauth_apps.count_active().await? == 0 {
                    return Err(AppError::soundcloud_temporarily_unavailable_for(Some(
                        OAUTH_APP_RETRY_SECONDS,
                    )));
                }
                return Ok(attempt(RefreshOutcome::ReauthorizationRequired, connection));
            }
            Err(error) => return Err(error),
        }
        if retry_pending(&connection) {
            return Ok(attempt(retry_outcome(&connection), connection));
        }
        let reply = self
            .core
            .refresh(&RefreshRequest {
                connection_id: connection.id,
                rejected_token_fingerprint: rejected_access_token.map(token_fingerprint),
                hold: false,
            })
            .await
            .map_err(|_| AppError::soundcloud_temporarily_unavailable())?;
        Ok(RefreshAttempt {
            outcome: reply.outcome,
            connection: Some(match reply.connection {
                Some(state) => merged(connection, state),
                None => connection,
            }),
            retry_after_sec: reply.retry_after_sec,
        })
    }
}

pub fn auth_session_response(session: &AuthSession) -> SoundCloudConnectionResponse {
    let status = session.connection_status();
    connection_status_response(status.as_ref(), session.any_oauth_app_active)
}

fn connection_response(connection: Option<&SoundCloudConnection>) -> SoundCloudConnectionResponse {
    let status = connection.map(SoundCloudConnectionStatus::from);
    connection_status_response(status.as_ref(), false)
}

fn connection_status_response(
    connection: Option<&SoundCloudConnectionStatus>,
    any_oauth_app_active: bool,
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
    let rejected_for_good =
        connection.last_refresh_error_kind.as_deref() == Some("reauthorization_required");
    let token_is_usable = expires_in > 0 && !token_is_rejected && !rejected_for_good;
    let refresh_is_due = expires_in <= REFRESH_DUE_BUFFER.as_secs() as i64 || token_is_rejected;
    let source_is_available = connection.has_refresh_credentials;
    let source_is_gone = !source_is_available && !token_is_usable && any_oauth_app_active;
    let retry_after = if lease_is_active {
        connection.refresh_lease_expires_at
    } else if retry_is_pending {
        connection.retry_at
    } else {
        None
    }
    .map(|retry_at| seconds_until(retry_at, now));
    let state = if rejected_for_good || source_is_gone {
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
        SoundCloudConnectionState::ReauthorizationRequired if !rejected_for_good => Some((
            "oauth_app_unavailable".to_owned(),
            OAUTH_APP_GONE.to_owned(),
        )),
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
    let retry_after = match &state {
        SoundCloudConnectionState::ReauthorizationRequired => None,
        SoundCloudConnectionState::RetryLater if !source_is_available => {
            retry_after.or(Some(OAUTH_APP_RETRY_SECONDS))
        }
        _ => retry_after,
    };

    SoundCloudConnectionResponse {
        state,
        can_use_soundcloud: token_is_usable,
        can_refresh: source_is_available
            && refresh_is_due
            && !lease_is_active
            && !retry_is_pending
            && !rejected_for_good,
        soundcloud_user_id: Some(connection.soundcloud_user_id.clone()),
        soundcloud_urn: Some(crate::common::sc_ids::user_urn(
            &connection.soundcloud_user_id,
        )),
        access_token_expires_at: Some(connection.expires_at),
        expires_in_sec: Some(expires_in),
        last_attempt_at: connection.last_refresh_attempt_at,
        last_success_at: connection.last_refresh_success_at,
        retry_after_sec: retry_after,
        error_code: active_error.as_ref().map(|(code, _)| code.clone()),
        error_message: active_error.map(|(_, message)| message),
    }
}

fn attempt(outcome: RefreshOutcome, connection: SoundCloudConnection) -> RefreshAttempt {
    RefreshAttempt {
        outcome,
        connection: Some(connection),
        retry_after_sec: None,
    }
}

fn merged(mut connection: SoundCloudConnection, state: ConnectionState) -> SoundCloudConnection {
    connection.access_token = state.access_token;
    connection.expires_at =
        DateTime::from_timestamp_millis(state.expires_at_unix_ms).unwrap_or(connection.expires_at);
    connection.scope = state.scope;
    connection.oauth_app_id = state.oauth_app_id;
    connection.refresh_generation = state.refresh_generation;
    connection.refresh_failure_count = state.refresh_failure_count;
    connection.refresh_lease_id = state.refresh_lease_id;
    connection.refresh_lease_expires_at = at(state.refresh_lease_expires_at_unix_ms);
    connection.last_refresh_attempt_at = at(state.last_refresh_attempt_at_unix_ms);
    connection.last_refresh_success_at = at(state.last_refresh_success_at_unix_ms);
    connection.last_refresh_error_kind = state.last_refresh_error_kind;
    connection.last_refresh_error = state.last_refresh_error;
    connection.retry_at = at(state.retry_at_unix_ms);
    connection
}

fn at(unix_ms: Option<i64>) -> Option<DateTime<Utc>> {
    unix_ms.and_then(DateTime::from_timestamp_millis)
}

fn retry_pending(connection: &SoundCloudConnection) -> bool {
    refresh_retry_pending(connection.retry_at)
}

fn refresh_lease_active(
    lease_id: Option<Uuid>,
    lease_expires_at: Option<DateTime<Utc>>,
) -> bool {
    lease_id.is_some() && lease_expires_at.is_some_and(|expires_at| expires_at > Utc::now())
}

fn refresh_retry_pending(retry_at: Option<DateTime<Utc>>) -> bool {
    retry_at.is_some_and(|retry_at| retry_at > Utc::now())
}

fn seconds_until(retry_at: DateTime<Utc>, now: DateTime<Utc>) -> i64 {
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

#[cfg(test)]
mod tests;
