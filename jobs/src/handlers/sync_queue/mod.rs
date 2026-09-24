mod actions;
mod client;
mod connection;
mod model;
#[cfg(test)]
mod playlist_tests;
mod repository;
mod storage;
#[cfg(test)]
mod track_tests;

use std::sync::Arc;

use backend_contracts::{SYNC_QUEUE_MAX_RETRIES, SyncQueueFlushPayload};
use futures::{StreamExt, stream};
use tracing::warn;

use crate::config::JobsConfig;
use crate::queue::{JobError, JobResult};

use self::actions::ActionError;
use self::client::{SoundCloudClient, SoundCloudError};
use self::model::ClaimedMutation;
use self::repository::{FinalizeError, SyncQueueRepository};
use self::storage::TrackStorage;

pub(super) use self::client::TokenRefreshClient;
pub(super) use self::connection::{ConnectionError, ConnectionManager};

const REAUTHORIZATION_RETRY_SECONDS: i64 = 15 * 60;
const BAN_RETRY_SECONDS: i64 = 30 * 60;
const RATE_LIMIT_RETRY_SECONDS: i64 = 5 * 60;
const RETRY_CAP_SECONDS: i64 = 60 * 60;
const MAX_DRAIN_BATCHES: usize = 32;

pub struct SyncQueueHandler {
    repository: Arc<SyncQueueRepository>,
    connections: Arc<ConnectionManager>,
    client: Arc<SoundCloudClient>,
    token_client: Arc<TokenRefreshClient>,
    storage: TrackStorage,
    concurrency: usize,
    claim_batch: i64,
}

impl SyncQueueHandler {
    pub fn new(config: &JobsConfig, pool: sqlx::PgPool) -> Result<Self, crate::ClientBuildError> {
        Ok(Self {
            repository: Arc::new(SyncQueueRepository::new(
                pool.clone(),
                config.sync_queue.lease_duration,
            )),
            connections: Arc::new(ConnectionManager::new(pool)),
            client: Arc::new(SoundCloudClient::new(&config.sync_queue)?),
            token_client: Arc::new(TokenRefreshClient::new(&config.oauth)?),
            storage: TrackStorage::new(&config.sync_queue)?,
            concurrency: config.sync_queue.concurrency,
            claim_batch: i64::try_from(config.sync_queue.claim_batch).unwrap_or(i64::MAX),
        })
    }

    pub async fn flush(&self, payload: SyncQueueFlushPayload) -> JobResult {
        if payload.force {
            self.repository
                .force_due()
                .await
                .map_err(JobError::retryable)?;
        }
        for _ in 0..MAX_DRAIN_BATCHES {
            let claimed = self.flush_batch().await?;
            if claimed < self.claim_batch as usize {
                break;
            }
        }
        Ok(())
    }

    async fn flush_batch(&self) -> JobResult<usize> {
        let mutations = self
            .repository
            .claim(self.claim_batch)
            .await
            .map_err(JobError::retryable)?;
        let claimed = mutations.len();
        let mut executions = stream::iter(mutations)
            .map(|mutation| self.execute(mutation))
            .buffer_unordered(self.concurrency);
        let mut infrastructure_error = None;
        while let Some(result) = executions.next().await {
            if let Err(error) = result {
                infrastructure_error.get_or_insert(error);
            }
        }
        infrastructure_error.map_or(Ok(claimed), |error| Err(JobError::retryable(error)))
    }

    async fn execute(&self, mutation: ClaimedMutation) -> Result<(), anyhow::Error> {
        if let Err(error) = self.storage.evict_private(&mutation).await {
            self.repository.release(&mutation).await?;
            return Err(error);
        }
        if mutation.has_remote_result() {
            return self.finalize(&mutation).await;
        }
        if is_non_idempotent(&mutation.action_type) && mutation.has_ambiguous_remote_attempt() {
            self.repository
                .park(
                    &mutation,
                    SYNC_QUEUE_MAX_RETRIES,
                    "Remote outcome is ambiguous; automatic retry is disabled",
                )
                .await?;
            return Ok(());
        }
        let token = match self
            .connections
            .access_token(&self.token_client, &mutation.user_id)
            .await
        {
            Ok(token) => token,
            Err(error) => {
                self.record_connection_failure(&mutation, &error).await?;
                return Ok(());
            }
        };
        if !self.repository.record_remote_attempt(&mutation).await? {
            self.repository.release(&mutation).await?;
            return Ok(());
        }
        let remote_result =
            match actions::execute_remote(&self.client, &mutation, &token.value).await {
                Ok(result) => result,
                Err(ActionError::SoundCloud(error)) if error.is_unauthorized() => {
                    let refreshed = match self
                        .connections
                        .refresh_rejected_token(&self.token_client, &mutation.user_id, &token.value)
                        .await
                    {
                        Ok(token) => token,
                        Err(error) => {
                            self.record_connection_failure(&mutation, &error).await?;
                            return Ok(());
                        }
                    };
                    match actions::execute_remote(&self.client, &mutation, &refreshed.value).await {
                        Ok(result) => result,
                        Err(ActionError::SoundCloud(error)) if error.is_unauthorized() => {
                            self.connections
                                .reject_for_later(&mutation.user_id, &refreshed.value)
                                .await?;
                            self.repository
                                .postpone_unattempted(
                                    &mutation,
                                    "SoundCloud rejected the refreshed access token",
                                    RATE_LIMIT_RETRY_SECONDS,
                                )
                                .await?;
                            return Ok(());
                        }
                        Err(error) => {
                            self.record_action_failure(&mutation, &error).await?;
                            return Ok(());
                        }
                    }
                }
                Err(error) => {
                    self.record_action_failure(&mutation, &error).await?;
                    return Ok(());
                }
            };
        if !self
            .repository
            .record_remote_success(&mutation, &remote_result)
            .await?
        {
            self.repository.release(&mutation).await?;
            return Ok(());
        }
        self.finalize(&mutation).await
    }

    async fn finalize(&self, mutation: &ClaimedMutation) -> Result<(), anyhow::Error> {
        match self.repository.finalize(mutation).await {
            Ok(()) => Ok(()),
            Err(FinalizeError::Action(
                error @ (ActionError::InvalidPayload(_)
                | ActionError::InvalidRemoteResult(_)
                | ActionError::UnknownAction(_)),
            )) => {
                self.repository
                    .park(mutation, SYNC_QUEUE_MAX_RETRIES, &error.to_string())
                    .await?;
                Ok(())
            }
            Err(error) => {
                self.repository.release(mutation).await?;
                Err(error.into())
            }
        }
    }

    async fn record_connection_failure(
        &self,
        mutation: &ClaimedMutation,
        error: &ConnectionError,
    ) -> Result<(), sqlx::Error> {
        let retry_after_seconds = match error {
            ConnectionError::ReauthorizationRequired => REAUTHORIZATION_RETRY_SECONDS,
            ConnectionError::RefreshInProgress {
                retry_after_seconds,
            }
            | ConnectionError::RateLimited {
                retry_after_seconds,
            }
            | ConnectionError::TemporarilyUnavailable {
                retry_after_seconds,
            } => *retry_after_seconds,
            ConnectionError::Database(error) => return Err(clone_database_error(error)),
        };
        if is_non_idempotent(&mutation.action_type) {
            self.repository
                .postpone_unattempted(mutation, &error.to_string(), retry_after_seconds)
                .await?;
        } else {
            self.repository
                .postpone(mutation, &error.to_string(), retry_after_seconds)
                .await?;
        }
        Ok(())
    }

    async fn record_action_failure(
        &self,
        mutation: &ClaimedMutation,
        error: &ActionError,
    ) -> Result<(), sqlx::Error> {
        if is_non_idempotent(&mutation.action_type) {
            return self.record_non_idempotent_failure(mutation, error).await;
        }
        match error {
            ActionError::SoundCloud(error) if error.is_banned() => {
                self.repository
                    .postpone(mutation, &error.to_string(), BAN_RETRY_SECONDS)
                    .await?;
            }
            ActionError::SoundCloud(error) if error.is_rate_limited() => {
                self.repository
                    .postpone(
                        mutation,
                        &error.to_string(),
                        error
                            .retry_after_seconds()
                            .unwrap_or(RATE_LIMIT_RETRY_SECONDS),
                    )
                    .await?;
            }
            ActionError::InvalidPayload(_)
            | ActionError::InvalidRemoteResult(_)
            | ActionError::UnknownAction(_) => {
                self.repository
                    .park(mutation, SYNC_QUEUE_MAX_RETRIES, &error.to_string())
                    .await?;
            }
            ActionError::Database(error) => return Err(clone_database_error(error)),
            ActionError::SoundCloud(error) => {
                let retry_count = mutation.retry_count.saturating_add(1);
                if retry_count >= SYNC_QUEUE_MAX_RETRIES {
                    self.repository
                        .park(mutation, retry_count, &error.to_string())
                        .await?;
                    warn!(
                        action = %mutation.action_type,
                        target = %mutation.target_urn,
                        retries = retry_count,
                        "sync action parked"
                    );
                } else {
                    self.repository
                        .retry(
                            mutation,
                            &error.to_string(),
                            retry_delay_seconds(retry_count),
                        )
                        .await?;
                }
            }
        }
        Ok(())
    }

    async fn record_non_idempotent_failure(
        &self,
        mutation: &ClaimedMutation,
        error: &ActionError,
    ) -> Result<(), sqlx::Error> {
        match error {
            ActionError::SoundCloud(error) if error.is_banned() => {
                self.repository
                    .postpone_unattempted(mutation, &error.to_string(), BAN_RETRY_SECONDS)
                    .await?;
            }
            ActionError::SoundCloud(error) if error.is_rate_limited() => {
                self.repository
                    .postpone_unattempted(
                        mutation,
                        &error.to_string(),
                        error
                            .retry_after_seconds()
                            .unwrap_or(RATE_LIMIT_RETRY_SECONDS),
                    )
                    .await?;
            }
            ActionError::Database(error) => return Err(clone_database_error(error)),
            ActionError::SoundCloud(SoundCloudError::Api { status, .. })
                if status.is_client_error() =>
            {
                self.repository
                    .park(mutation, SYNC_QUEUE_MAX_RETRIES, &error.to_string())
                    .await?;
            }
            ActionError::InvalidPayload(_)
            | ActionError::InvalidRemoteResult(_)
            | ActionError::UnknownAction(_) => {
                self.repository
                    .park(mutation, SYNC_QUEUE_MAX_RETRIES, &error.to_string())
                    .await?;
            }
            ActionError::SoundCloud(_) => {
                self.repository
                    .park(
                        mutation,
                        SYNC_QUEUE_MAX_RETRIES,
                        "Remote outcome is ambiguous; automatic retry is disabled",
                    )
                    .await?;
            }
        }
        Ok(())
    }
}

fn retry_delay_seconds(retry_count: i32) -> i64 {
    let exponent = u32::try_from(retry_count.max(0)).map_or(0, |value| value.min(10));
    60_i64
        .saturating_mul(1_i64.checked_shl(exponent).unwrap_or(i64::MAX))
        .min(RETRY_CAP_SECONDS)
}

fn is_non_idempotent(action_type: &str) -> bool {
    matches!(action_type, "comment" | "playlist_create")
}

fn clone_database_error(error: &sqlx::Error) -> sqlx::Error {
    sqlx::Error::Protocol(error.to_string())
}

#[cfg(test)]
mod tests;
