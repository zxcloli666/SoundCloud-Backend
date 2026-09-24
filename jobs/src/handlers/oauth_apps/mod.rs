mod client;
mod model;
mod repository;

use std::time::Duration;

use chrono::{DateTime, Utc};
use futures::{StreamExt, stream};
use sqlx::PgPool;
use tracing::{info, warn};

use crate::config::{OAuthAppBootstrap, OAuthConfig};
use crate::handlers::oauth_cooldowns::OAuthAppCooldowns;
use crate::queue::{JobError, JobResult};

use self::client::{OAuthTokenClient, TokenRequestOutcome};
use self::model::{ClaimedApp, Token};
use self::repository::OAuthRefreshRepository;

const CLAIM_BATCH: i64 = 8;
const REFRESH_CONCURRENCY: usize = 4;
const REFRESH_LEASE: Duration = Duration::from_secs(120);
const MIN_BACKOFF: Duration = Duration::from_secs(30);
const MAX_BACKOFF: Duration = Duration::from_secs(15 * 60);

pub struct OAuthAppsRefreshHandler {
    repository: OAuthRefreshRepository,
    cooldowns: OAuthAppCooldowns,
    client: OAuthTokenClient,
    bootstrap_app: Option<OAuthAppBootstrap>,
}

impl OAuthAppsRefreshHandler {
    pub fn new(pool: PgPool, config: &OAuthConfig) -> Result<Self, crate::ClientBuildError> {
        Ok(Self {
            repository: OAuthRefreshRepository::new(pool.clone()),
            cooldowns: OAuthAppCooldowns::new(pool),
            client: OAuthTokenClient::new(config)?,
            bootstrap_app: config.bootstrap_app.clone(),
        })
    }

    pub async fn bootstrap(&self) -> JobResult {
        let Some(app) = self.bootstrap_app.as_ref() else {
            return Ok(());
        };
        let app_id = self
            .repository
            .bootstrap_app(app)
            .await
            .map_err(JobError::retryable)?;
        info!(%app_id, app_name = %app.name, "configured OAuth app ready");
        Ok(())
    }

    pub async fn refresh_due(&self) -> JobResult {
        let claims = self
            .repository
            .claim_due(CLAIM_BATCH, REFRESH_LEASE)
            .await
            .map_err(JobError::retryable)?;
        let mut refreshes = stream::iter(claims)
            .map(|claim| self.refresh_claim(claim))
            .buffer_unordered(REFRESH_CONCURRENCY);

        let mut failure = None;
        while let Some(result) = refreshes.next().await {
            if let Err(error) = result {
                failure.get_or_insert(error);
            }
        }
        failure.map_or(Ok(()), Err)
    }

    async fn refresh_claim(&self, claim: ClaimedApp) -> JobResult {
        if let Some(retry_after_seconds) = self
            .cooldowns
            .retry_after_seconds(claim.id)
            .await
            .map_err(JobError::retryable)?
        {
            return self
                .retry_at(
                    &claim,
                    "app_cooldown",
                    Utc::now() + chrono::Duration::seconds(retry_after_seconds),
                )
                .await;
        }
        match claim.refresh_token.as_deref() {
            Some(refresh_token) => match self.client.refresh(&claim, refresh_token).await {
                TokenRequestOutcome::RefreshSuccess(token) => self.complete(&claim, token).await,
                TokenRequestOutcome::ClientCredentialsSuccess(_) => {
                    self.retry(&claim, "refresh_incomplete", None).await
                }
                TokenRequestOutcome::Rejected(rejection) if rejection.invalid_grant() => {
                    self.client_credentials(&claim).await
                }
                TokenRequestOutcome::Incomplete => self.client_credentials(&claim).await,
                TokenRequestOutcome::Rejected(rejection) if rejection.shared_cooldown() => {
                    self.retry_shared(
                        &claim,
                        "refresh_unavailable",
                        rejection.minimum_cooldown_seconds(),
                        rejection.retry_at,
                    )
                    .await
                }
                TokenRequestOutcome::Rejected(rejection) => {
                    self.retry(&claim, "refresh_rejected", rejection.retry_at)
                        .await
                }
                TokenRequestOutcome::Unavailable => {
                    self.retry_shared(&claim, "refresh_unavailable", 30, None)
                        .await
                }
            },
            None => self.client_credentials(&claim).await,
        }
    }

    async fn client_credentials(&self, claim: &ClaimedApp) -> JobResult {
        let reservation = self
            .repository
            .reserve_client_credentials(claim)
            .await
            .map_err(JobError::retryable)?;
        let Some(reservation_id) = reservation.reservation_id else {
            return self
                .retry_at(claim, "client_credentials_limited", reservation.retry_at)
                .await;
        };

        match self.client.client_credentials(claim).await {
            TokenRequestOutcome::ClientCredentialsSuccess(token) => {
                self.complete(claim, token).await
            }
            TokenRequestOutcome::RefreshSuccess(_) => {
                self.retry(claim, "client_credentials_incomplete", None)
                    .await
            }
            TokenRequestOutcome::Rejected(rejection) => {
                self.repository
                    .release_reservation(reservation_id)
                    .await
                    .map_err(JobError::retryable)?;
                if rejection.shared_cooldown() {
                    self.retry_shared(
                        claim,
                        "client_credentials_unavailable",
                        rejection.minimum_cooldown_seconds(),
                        rejection.retry_at,
                    )
                    .await
                } else {
                    self.retry(claim, "client_credentials_rejected", rejection.retry_at)
                        .await
                }
            }
            TokenRequestOutcome::Incomplete => {
                self.retry_shared(claim, "client_credentials_incomplete", 30, None)
                    .await
            }
            TokenRequestOutcome::Unavailable => {
                self.retry_shared(claim, "client_credentials_unavailable", 30, None)
                    .await
            }
        }
    }

    async fn complete(&self, claim: &ClaimedApp, token: Token) -> JobResult {
        let completed = self
            .repository
            .complete(claim, &token)
            .await
            .map_err(JobError::retryable)?;
        if !completed {
            warn!(app_id = %claim.id, "OAuth token refresh lease expired before completion");
        }
        Ok(())
    }

    async fn retry(
        &self,
        claim: &ClaimedApp,
        reason: &'static str,
        retry_at: Option<DateTime<Utc>>,
    ) -> JobResult {
        let backoff_at = Utc::now() + failure_backoff(claim.refresh_attempts());
        let retry_at = retry_at.map_or(backoff_at, |upstream| upstream.max(backoff_at));
        self.retry_at(claim, reason, retry_at).await
    }

    async fn retry_shared(
        &self,
        claim: &ClaimedApp,
        reason: &'static str,
        minimum_seconds: i64,
        upstream_retry_at: Option<DateTime<Utc>>,
    ) -> JobResult {
        let local_retry_at = Utc::now() + failure_backoff(claim.refresh_attempts());
        let retry_at =
            upstream_retry_at.map_or(local_retry_at, |upstream| upstream.max(local_retry_at));
        let upstream_seconds = seconds_until(retry_at);
        let retry_after_seconds = self
            .cooldowns
            .penalize(claim.id, minimum_seconds.max(upstream_seconds))
            .await
            .map_err(JobError::retryable)?;
        self.retry_at(
            claim,
            reason,
            Utc::now() + chrono::Duration::seconds(retry_after_seconds),
        )
        .await
    }

    async fn retry_at(
        &self,
        claim: &ClaimedApp,
        reason: &'static str,
        retry_at: DateTime<Utc>,
    ) -> JobResult {
        let recorded = self
            .repository
            .retry(claim, retry_at, reason)
            .await
            .map_err(JobError::retryable)?;
        if recorded {
            warn!(app_id = %claim.id, reason, retry_at = %retry_at, "OAuth token refresh postponed");
        }
        Ok(())
    }
}

fn failure_backoff(refresh_attempts: i32) -> chrono::Duration {
    let exponent = u32::try_from(refresh_attempts.max(0)).map_or(0, |value| value.min(5));
    let seconds = MIN_BACKOFF
        .saturating_mul(2_u32.saturating_pow(exponent))
        .min(MAX_BACKOFF)
        .as_secs();
    chrono::Duration::seconds(i64::try_from(seconds).unwrap_or(i64::MAX))
}

fn seconds_until(deadline: DateTime<Utc>) -> i64 {
    deadline
        .signed_duration_since(Utc::now())
        .num_milliseconds()
        .max(1)
        .saturating_add(999)
        / 1_000
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failures_back_off_without_exceeding_the_cap() {
        assert!(failure_backoff(1) > failure_backoff(0));
        assert_eq!(failure_backoff(i32::MAX), chrono::Duration::minutes(15));
    }
}
