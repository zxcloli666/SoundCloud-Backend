use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use backend_contracts::auth::{
    AUTH_APP_TOKEN_SUBJECT, AUTH_REFRESH_SUBJECT, AUTH_RPC_QUEUE, AppToken, AppTokenReply,
    AppTokenRequest, ConnectionState, RefreshOutcome, RefreshReply, RefreshRequest,
    token_fingerprint,
};
use chrono::{DateTime, Utc};
use sqlx::PgPool;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::bus::Bus;
use crate::config::OAuthConfig;

use super::sync_queue::{Connection, ConnectionError, ConnectionManager, TokenRefreshClient};

const CONCURRENCY: usize = 64;
const TURN_WAIT: Duration = Duration::from_secs(4);
const LEASE_WAIT: Duration = Duration::from_secs(4);
const LEASE_POLL: Duration = Duration::from_millis(200);
const REFRESH_DUE_BUFFER: chrono::Duration = chrono::Duration::minutes(5);
const REJECTED_TOKEN_RETRY_SECONDS: i32 = 5 * 60;

pub struct AuthRpc {
    pool: PgPool,
    connections: ConnectionManager,
    client: TokenRefreshClient,
    turns: Turns,
}

#[derive(Default)]
struct Turns(Mutex<HashMap<Uuid, Arc<tokio::sync::Mutex<()>>>>);

impl Turns {
    fn of(&self, connection_id: Uuid) -> Arc<tokio::sync::Mutex<()>> {
        let mut turns = self.0.lock().unwrap_or_else(|error| error.into_inner());
        turns.retain(|_, turn| Arc::strong_count(turn) > 1);
        turns.entry(connection_id).or_default().clone()
    }
}

impl AuthRpc {
    pub fn new(pool: PgPool, oauth: &OAuthConfig) -> Result<Self, wreq::Error> {
        Ok(Self {
            connections: ConnectionManager::new(pool.clone()),
            client: TokenRefreshClient::new(oauth)?,
            pool,
            turns: Turns::default(),
        })
    }

    pub async fn serve(self: Arc<Self>, bus: Bus, cancellation: CancellationToken) -> anyhow::Result<()> {
        let refresh = self.clone();
        let app_token = self;
        tokio::try_join!(
            bus.serve(
                AUTH_REFRESH_SUBJECT,
                AUTH_RPC_QUEUE,
                CONCURRENCY,
                cancellation.clone(),
                move |request: RefreshRequest| {
                    let rpc = refresh.clone();
                    async move { rpc.refresh(request).await }
                },
            ),
            bus.serve(
                AUTH_APP_TOKEN_SUBJECT,
                AUTH_RPC_QUEUE,
                CONCURRENCY,
                cancellation,
                move |request: AppTokenRequest| {
                    let rpc = app_token.clone();
                    async move { rpc.app_token(request).await }
                },
            ),
        )?;
        Ok(())
    }

    pub async fn refresh(&self, request: RefreshRequest) -> anyhow::Result<RefreshReply> {
        let turn = self.turns.of(request.connection_id);
        let Ok(_turn) = tokio::time::timeout(TURN_WAIT, turn.lock()).await else {
            return self.reply(request.connection_id, RefreshOutcome::InProgress, None).await;
        };
        let Some(connection) = self.connections.load_by_id(request.connection_id).await? else {
            return Ok(RefreshReply {
                outcome: RefreshOutcome::RetryLater,
                connection: None,
                retry_after_sec: Some(1),
            });
        };
        let rejected = request
            .rejected_token_fingerprint
            .as_deref()
            .is_some_and(|fingerprint| fingerprint == token_fingerprint(&connection.access_token));
        if request.hold {
            if rejected {
                self.connections
                    .mark_rejected(&connection, &connection.access_token, REJECTED_TOKEN_RETRY_SECONDS)
                    .await?;
            }
            let outcome = self.settled(connection.id).await?;
            return self.reply(connection.id, outcome, None).await;
        }
        let connection = if rejected && connection.token_is_usable() {
            self.connections
                .mark_rejected(&connection, &connection.access_token, 0)
                .await?;
            self.connections
                .load_by_id(connection.id)
                .await?
                .unwrap_or(connection)
        } else {
            connection
        };
        if connection.token_is_usable() && !refresh_due(&connection) {
            return self.reply(connection.id, RefreshOutcome::AlreadyFresh, None).await;
        }
        let connection_id = connection.id;
        let (outcome, retry_after) = match self.connections.refresh(&self.client, connection).await {
            Ok(_) => (RefreshOutcome::Refreshed, None),
            Err(ConnectionError::ReauthorizationRequired) => {
                (RefreshOutcome::ReauthorizationRequired, None)
            }
            Err(ConnectionError::RefreshInProgress { .. }) => {
                (self.after_lease(connection_id).await?, None)
            }
            Err(ConnectionError::RateLimited {
                retry_after_seconds,
            }) => (RefreshOutcome::RateLimited, Some(retry_after_seconds)),
            Err(ConnectionError::TemporarilyUnavailable {
                retry_after_seconds,
            }) => (self.settled(connection_id).await?, Some(retry_after_seconds)),
            Err(ConnectionError::Database(error)) => return Err(error.into()),
        };
        self.reply(connection_id, outcome, retry_after).await
    }

    pub async fn app_token(&self, request: AppTokenRequest) -> anyhow::Result<AppTokenReply> {
        let token = sqlx::query_file!("queries/auth_rpc/app_token.sql", request.oauth_app_id)
            .fetch_optional(&self.pool)
            .await?
            .map(|row| AppToken {
                generation: row.generation,
                access_token: row.access_token,
                expires_at_unix_ms: row.expires_at.timestamp_millis(),
            });
        Ok(AppTokenReply { token })
    }

    async fn after_lease(&self, connection_id: Uuid) -> anyhow::Result<RefreshOutcome> {
        let deadline = Instant::now() + LEASE_WAIT;
        loop {
            let outcome = self.settled(connection_id).await?;
            if outcome != RefreshOutcome::InProgress || Instant::now() >= deadline {
                return Ok(outcome);
            }
            tokio::time::sleep(LEASE_POLL).await;
        }
    }

    async fn settled(&self, connection_id: Uuid) -> anyhow::Result<RefreshOutcome> {
        let Some(connection) = self.connections.load_by_id(connection_id).await? else {
            return Ok(RefreshOutcome::RetryLater);
        };
        Ok(outcome_of(&connection))
    }

    async fn reply(
        &self,
        connection_id: Uuid,
        outcome: RefreshOutcome,
        retry_after_sec: Option<i64>,
    ) -> anyhow::Result<RefreshReply> {
        let connection = sqlx::query_file!("queries/auth_rpc/connection_state.sql", connection_id)
            .fetch_optional(&self.pool)
            .await?
            .map(|row| ConnectionState {
                access_token: row.access_token,
                expires_at_unix_ms: row.expires_at.timestamp_millis(),
                scope: row.scope,
                oauth_app_id: row.oauth_app_id,
                refresh_generation: row.refresh_generation,
                refresh_failure_count: row.refresh_failure_count,
                refresh_lease_id: row.refresh_lease_id,
                refresh_lease_expires_at_unix_ms: unix_ms(row.refresh_lease_expires_at),
                last_refresh_attempt_at_unix_ms: unix_ms(row.last_refresh_attempt_at),
                last_refresh_success_at_unix_ms: unix_ms(row.last_refresh_success_at),
                last_refresh_error_kind: row.last_refresh_error_kind,
                last_refresh_error: row.last_refresh_error,
                retry_at_unix_ms: unix_ms(row.retry_at),
            });
        let outcome = if connection.is_none() {
            RefreshOutcome::RetryLater
        } else {
            outcome
        };
        Ok(RefreshReply {
            outcome,
            connection,
            retry_after_sec,
        })
    }
}

fn outcome_of(connection: &Connection) -> RefreshOutcome {
    if connection.token_is_usable() && !refresh_due(connection) {
        return RefreshOutcome::AlreadyFresh;
    }
    if connection.last_refresh_error_kind.as_deref() == Some("reauthorization_required") {
        return RefreshOutcome::ReauthorizationRequired;
    }
    if connection.active_refresh_lease_seconds().is_some() {
        return RefreshOutcome::InProgress;
    }
    if connection.retry_seconds().is_some() {
        return match connection.last_refresh_error_kind.as_deref() {
            Some("rate_limited") => RefreshOutcome::RateLimited,
            Some("timed_out") => RefreshOutcome::TimedOut,
            _ => RefreshOutcome::RetryLater,
        };
    }
    if connection.token_is_usable() {
        return RefreshOutcome::AlreadyFresh;
    }
    RefreshOutcome::RetryLater
}

fn refresh_due(connection: &Connection) -> bool {
    connection.expires_at <= Utc::now() + REFRESH_DUE_BUFFER
}

fn unix_ms(at: Option<DateTime<Utc>>) -> Option<i64> {
    at.map(|at| at.timestamp_millis())
}

#[cfg(test)]
#[path = "auth_rpc_tests.rs"]
mod tests;
