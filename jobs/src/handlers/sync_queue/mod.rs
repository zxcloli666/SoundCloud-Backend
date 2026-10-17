mod actions;
mod client;
mod connection;
mod create_check;
mod model;
#[cfg(test)]
mod playlist_tests;
#[cfg(test)]
mod refusal_tests;
mod repository;
mod storage;
#[cfg(test)]
mod track_tests;
#[cfg(test)]
mod unclear_create_tests;
mod write_pause;

use std::sync::Arc;
use std::time::{Duration, Instant};

use backend_contracts::{SYNC_QUEUE_MAX_RETRIES, SyncQueueFlushPayload};
use futures::{StreamExt, stream};
use tracing::{info, warn};

use crate::config::JobsConfig;
use crate::handlers::star_priority::{Round, Turns};
use crate::queue::{JobError, JobResult};

use self::actions::{ActionError, Remote};
use self::client::{SoundCloudClient, SoundCloudError};
use self::model::ClaimedMutation;
use self::repository::{FinalizeError, PACED_ACTIONS, SyncQueueRepository};
use self::storage::TrackStorage;
use self::write_pause::{Admission, MAX_PAUSE, WritePauses};

pub(super) use self::client::TokenRefreshClient;
pub(super) use self::connection::{AccessToken, ConnectionError, ConnectionManager};

const REAUTHORIZATION_RETRY_SECONDS: i64 = 15 * 60;
const BAN_RETRY_SECONDS: i64 = 30 * 60;
const RATE_LIMIT_RETRY_SECONDS: i64 = 5 * 60;
const WRITE_PAUSE_SECONDS: i64 = 60;
const ACCOUNT_LIMIT_SECONDS: i64 = 30 * 60;
const SUBSCRIPTION_RETRY_SECONDS: i64 = 2 * 60 * 60;
const PAUSED_ACTION_RECHECK_SECONDS: i64 = 15 * 60;
const UNCLEAR_CREATE_RETRY_SECONDS: i64 = 2 * 60;
const INFRASTRUCTURE_RETRY_SECONDS: i64 = 60;
const RETRY_CAP_SECONDS: i64 = 60 * 60;
const MAX_DRAIN_BATCHES: usize = 32;
const PACED_DRAIN_BUDGET: Duration = Duration::from_secs(50);

pub struct SyncQueueHandler {
    repository: Arc<SyncQueueRepository>,
    connections: Arc<ConnectionManager>,
    client: Arc<SoundCloudClient>,
    token_client: Arc<TokenRefreshClient>,
    storage: TrackStorage,
    write_pauses: WritePauses<(uuid::Uuid, String)>,
    account_pauses: WritePauses<(String, String)>,
    like_interval: Duration,
    pauses_restored: tokio::sync::OnceCell<()>,
    concurrency: usize,
    claim_batch: i64,
    turns: Turns,
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
            write_pauses: WritePauses::default(),
            account_pauses: WritePauses::default(),
            like_interval: config.sync_queue.like_interval,
            pauses_restored: tokio::sync::OnceCell::new(),
            concurrency: config.sync_queue.concurrency,
            claim_batch: i64::try_from(config.sync_queue.claim_batch).unwrap_or(i64::MAX),
            turns: Turns::default(),
        })
    }

    pub async fn flush(&self, payload: SyncQueueFlushPayload) -> JobResult {
        if payload.force {
            self.repository
                .force_due()
                .await
                .map_err(JobError::retryable)?;
        }
        self.pauses_restored
            .get_or_init(|| self.restore_pauses())
            .await;
        let deadline = Instant::now() + PACED_DRAIN_BUDGET;
        let mut round = Round::new("sync_queue");
        for _ in 0..MAX_DRAIN_BATCHES {
            let claimed = self.flush_batch(&mut round).await?;
            if claimed >= self.claim_batch as usize {
                continue;
            }
            match self.write_pauses.next_spacing_end() {
                Some(wait) if Instant::now() + wait < deadline => tokio::time::sleep(wait).await,
                _ => break,
            }
        }
        let next = self
            .repository
            .next_waiting(&self.held_actions())
            .await
            .map_err(JobError::retryable)?;
        round.record("star", 0, next.star);
        round.record("regular", 0, next.regular);
        round.report();
        Ok(())
    }

    fn held_actions(&self) -> Vec<String> {
        self.write_pauses
            .held()
            .into_iter()
            .map(|(oauth_app_id, action)| format!("{oauth_app_id}:{action}"))
            .collect()
    }

    async fn flush_batch(&self, round: &mut Round) -> JobResult<usize> {
        let mutations = self
            .repository
            .claim_outside(
                self.claim_batch,
                &self.held_actions(),
                self.turns.next().star_first(),
            )
            .await
            .map_err(JobError::retryable)?;
        let star = mutations.iter().filter(|mutation| mutation.star).count();
        round.record("star", star, None);
        round.record("regular", mutations.len() - star, None);
        let claimed = mutations.len();
        let mut executions = stream::iter(mutations)
            .map(|mutation| self.execute(mutation))
            .buffer_unordered(self.concurrency);
        let mut handled = 0;
        let mut infrastructure_error = None;
        while let Some(result) = executions.next().await {
            match result {
                Ok(()) => handled += 1,
                Err(error) => {
                    warn!(error = %error, "sync mutation failed");
                    infrastructure_error.get_or_insert(error);
                }
            }
        }
        match infrastructure_error {
            Some(error) if handled == 0 => Err(JobError::retryable(error)),
            _ => Ok(claimed),
        }
    }

    async fn execute(&self, mutation: ClaimedMutation) -> Result<(), anyhow::Error> {
        if let Err(error) = self.storage.evict_private(&mutation).await {
            self.postpone_after_failure(&mutation, &error.to_string())
                .await?;
            return Ok(());
        }
        if mutation.has_remote_result() {
            return self.finalize(&mutation).await;
        }
        if mutation.action_type == "comment" && mutation.has_ambiguous_remote_attempt() {
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
        let token = if mutation.action_type == "playlist_create"
            && mutation.has_ambiguous_remote_attempt()
        {
            let Some(token) = self.confirm_unclear_create(&mutation, token).await? else {
                return Ok(());
            };
            token
        } else {
            token
        };
        if let Some(oauth_app_id) = token.oauth_app_id {
            let action = (oauth_app_id, mutation.action_type.clone());
            match self.write_pauses.admit(&action) {
                Admission::Paused(seconds) => {
                    self.postpone_before_post(
                        &mutation,
                        "SoundCloud paused this action for the application",
                        seconds.min(PAUSED_ACTION_RECHECK_SECONDS),
                    )
                    .await?;
                    return Ok(());
                }
                Admission::Open => {
                    if let Some(spacing) = self.spacing(&mutation.action_type) {
                        self.write_pauses
                            .pause(action, i64::try_from(spacing.as_secs()).unwrap_or(1));
                    }
                }
                Admission::Probe => {}
            }
        }
        if let Some(seconds) = self
            .account_pauses
            .remaining_seconds(&account_action(&mutation))
        {
            self.postpone_before_post(
                &mutation,
                "SoundCloud limits this action for the account",
                seconds.min(PAUSED_ACTION_RECHECK_SECONDS),
            )
            .await?;
            return Ok(());
        }
        if !self.repository.record_remote_attempt(&mutation).await? {
            self.repository.release(&mutation).await?;
            return Ok(());
        }
        let (remote, oauth_app_id) =
            match actions::execute_remote(&self.client, &mutation, &token.value).await {
                Ok(remote) => (remote, token.oauth_app_id),
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
                        Ok(remote) => (remote, refreshed.oauth_app_id),
                        Err(ActionError::SoundCloud(error)) if error.is_unauthorized() => {
                            self.connections
                                .reject_for_later(&mutation.user_id, &refreshed.value)
                                .await?;
                            self.postpone_before_post(
                                &mutation,
                                "SoundCloud rejected the refreshed access token",
                                RATE_LIMIT_RETRY_SECONDS,
                            )
                            .await?;
                            return Ok(());
                        }
                        Err(error) => {
                            self.pause_writes_when_refused(
                                &mutation,
                                &error,
                                refreshed.oauth_app_id,
                            )
                            .await;
                            self.record_action_failure(&mutation, &error).await?;
                            return Ok(());
                        }
                    }
                }
                Err(error) => {
                    self.pause_writes_when_refused(&mutation, &error, token.oauth_app_id)
                        .await;
                    self.record_action_failure(&mutation, &error).await?;
                    return Ok(());
                }
            };
        let remote_result = match remote {
            Remote::Written(result) => {
                if let Some(oauth_app_id) = oauth_app_id {
                    self.write_pauses.succeeded(
                        &(oauth_app_id, mutation.action_type.clone()),
                        self.spacing(&mutation.action_type),
                    );
                    self.forget_pauses(oauth_app_id, &mutation.action_type)
                        .await;
                }
                result
            }
            Remote::TargetMissing => serde_json::Value::Null,
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

    async fn confirm_unclear_create(
        &self,
        mutation: &ClaimedMutation,
        mut token: AccessToken,
    ) -> Result<Option<AccessToken>, anyhow::Error> {
        let owned = self.repository.owned_playlist_urns(mutation).await?;
        let mut checked = create_check::confirm(&self.client, &token.value, mutation, &owned).await;
        if matches!(&checked, Err(ActionError::SoundCloud(error)) if error.is_unauthorized()) {
            token = match self
                .connections
                .refresh_rejected_token(&self.token_client, &mutation.user_id, &token.value)
                .await
            {
                Ok(token) => token,
                Err(error) => {
                    self.record_connection_failure(mutation, &error).await?;
                    return Ok(None);
                }
            };
            checked = create_check::confirm(&self.client, &token.value, mutation, &owned).await;
        }
        match checked {
            Ok(None) => {
                info!(
                    target = %mutation.target_urn,
                    "the unclear playlist create is not on soundcloud, sending it again"
                );
                Ok(Some(token))
            }
            Ok(Some(playlist)) => {
                info!(
                    target = %mutation.target_urn,
                    playlist = playlist.get("urn").and_then(serde_json::Value::as_str).unwrap_or_default(),
                    "the unclear playlist create is already on soundcloud"
                );
                if self
                    .repository
                    .record_remote_success(mutation, &playlist)
                    .await?
                {
                    self.finalize(mutation).await?;
                } else {
                    self.repository.release(mutation).await?;
                }
                Ok(None)
            }
            Err(error @ ActionError::InvalidPayload(_)) => {
                self.repository
                    .park(mutation, SYNC_QUEUE_MAX_RETRIES, &error.to_string())
                    .await?;
                Ok(None)
            }
            Err(error) => {
                self.retry_unclear_create(
                    mutation,
                    &format!("checking SoundCloud for the created playlist: {error}"),
                )
                .await?;
                Ok(None)
            }
        }
    }

    async fn postpone_before_post(
        &self,
        mutation: &ClaimedMutation,
        message: &str,
        retry_after_seconds: i64,
    ) -> Result<bool, sqlx::Error> {
        if mutation.has_ambiguous_remote_attempt() {
            self.repository
                .postpone(mutation, message, retry_after_seconds)
                .await
        } else {
            self.repository
                .postpone_unattempted(mutation, message, retry_after_seconds)
                .await
        }
    }

    async fn retry_unclear_create(
        &self,
        mutation: &ClaimedMutation,
        message: &str,
    ) -> Result<(), sqlx::Error> {
        let retry_count = mutation.retry_count.saturating_add(1);
        if retry_count >= SYNC_QUEUE_MAX_RETRIES {
            self.repository.park(mutation, retry_count, message).await?;
            warn!(
                target = %mutation.target_urn,
                retries = retry_count,
                "unclear playlist create parked"
            );
        } else {
            self.repository
                .retry(mutation, message, UNCLEAR_CREATE_RETRY_SECONDS)
                .await?;
        }
        Ok(())
    }

    async fn restore_pauses(&self) {
        if let Err(error) = self.repository.cap_action_pauses(MAX_PAUSE).await {
            warn!(%error, "saved action pauses were not cut to the longest pause");
        }
        let pauses = match self.repository.action_pauses().await {
            Ok(pauses) => pauses,
            Err(error) => {
                warn!(%error, "saved action pauses were not loaded, refused actions start a new backoff");
                return;
            }
        };
        for (oauth_app_id, action_type, remaining_seconds, strikes) in pauses {
            let remaining = Duration::from_secs(u64::try_from(remaining_seconds).unwrap_or(0));
            self.write_pauses
                .restore((oauth_app_id, action_type), remaining, strikes);
        }
    }

    async fn forget_pauses(&self, oauth_app_id: uuid::Uuid, action_type: &str) {
        let cleared = self
            .repository
            .clear_action_pauses(oauth_app_id, action_type)
            .await;
        match cleared {
            Ok(lifted) => {
                for lifted in lifted.into_iter().filter(|lifted| lifted != action_type) {
                    self.write_pauses.succeeded(&(oauth_app_id, lifted), None);
                }
            }
            Err(error) => {
                warn!(%error, "lifted action pauses were not removed from the database");
            }
        }
    }

    fn spacing(&self, action_type: &str) -> Option<Duration> {
        PACED_ACTIONS
            .contains(&action_type)
            .then_some(self.like_interval)
    }

    async fn pause_writes_when_refused(
        &self,
        mutation: &ClaimedMutation,
        error: &ActionError,
        oauth_app_id: Option<uuid::Uuid>,
    ) {
        let ActionError::SoundCloud(error) = error else {
            return;
        };
        if error.is_account_rate_limited() {
            warn!(
                user_id = %mutation.user_id,
                action = %mutation.action_type,
                seconds = ACCOUNT_LIMIT_SECONDS,
                "soundcloud limits this action for the account, holding its queue"
            );
            self.account_pauses
                .pause(account_action(mutation), ACCOUNT_LIMIT_SECONDS);
            return;
        }
        let Some(oauth_app_id) = oauth_app_id else {
            return;
        };
        let action = (oauth_app_id, mutation.action_type.clone());
        let (seconds, strikes, reason) = if error.is_subscription_required() {
            let (seconds, strikes) = self
                .write_pauses
                .pause_refused(action, SUBSCRIPTION_RETRY_SECONDS);
            (seconds, strikes, "subscription_required")
        } else if let Some(seconds) = error
            .retry_after_seconds()
            .filter(|_| error.is_rate_limited())
        {
            let (seconds, strikes) = self.write_pauses.pause_refused(action, seconds);
            (seconds, strikes, "rate_limited")
        } else if error.is_rate_limited() {
            let (seconds, strikes) = self
                .write_pauses
                .pause_longer_each_time(action, WRITE_PAUSE_SECONDS);
            (seconds, strikes, "rate_limited")
        } else {
            return;
        };
        let saved = self
            .repository
            .save_action_pause(
                oauth_app_id,
                &mutation.action_type,
                seconds,
                strikes,
                reason,
            )
            .await;
        if let Err(error) = saved {
            warn!(%error, "a refused action pause was not saved and will not survive a restart");
        }
        warn!(
            %oauth_app_id,
            action = %mutation.action_type,
            seconds,
            response = %error.response_excerpt(),
            "soundcloud refused this action for the whole application, pausing it"
        );
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
                self.postpone_after_failure(mutation, &error.to_string())
                    .await?;
                Ok(())
            }
        }
    }

    async fn postpone_after_failure(
        &self,
        mutation: &ClaimedMutation,
        message: &str,
    ) -> Result<(), sqlx::Error> {
        warn!(
            action = %mutation.action_type,
            target = %mutation.target_urn,
            error = message,
            "sync action postponed"
        );
        self.repository
            .postpone(mutation, message, INFRASTRUCTURE_RETRY_SECONDS)
            .await?;
        Ok(())
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
            self.postpone_before_post(mutation, &error.to_string(), retry_after_seconds)
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
            ActionError::SoundCloud(error) if error.is_subscription_required() => {
                self.repository
                    .postpone_unattempted(
                        mutation,
                        &error.to_string(),
                        PAUSED_ACTION_RECHECK_SECONDS,
                    )
                    .await?;
            }
            ActionError::SoundCloud(error) if error.is_account_rate_limited() => {
                self.repository
                    .postpone_unattempted(
                        mutation,
                        &error.to_string(),
                        PAUSED_ACTION_RECHECK_SECONDS,
                    )
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
            ActionError::SoundCloud(error) if error.is_subscription_required() => {
                self.repository
                    .postpone_unattempted(
                        mutation,
                        &error.to_string(),
                        PAUSED_ACTION_RECHECK_SECONDS,
                    )
                    .await?;
            }
            ActionError::SoundCloud(error) if error.is_account_rate_limited() => {
                self.repository
                    .postpone_unattempted(
                        mutation,
                        &error.to_string(),
                        PAUSED_ACTION_RECHECK_SECONDS,
                    )
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
            ActionError::SoundCloud(error) if mutation.action_type == "playlist_create" => {
                self.retry_unclear_create(mutation, &error.to_string())
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

fn account_action(mutation: &ClaimedMutation) -> (String, String) {
    (mutation.user_id.clone(), mutation.action_type.clone())
}

fn is_non_idempotent(action_type: &str) -> bool {
    matches!(action_type, "comment" | "playlist_create")
}

fn clone_database_error(error: &sqlx::Error) -> sqlx::Error {
    sqlx::Error::Protocol(error.to_string())
}

#[cfg(test)]
mod tests;
