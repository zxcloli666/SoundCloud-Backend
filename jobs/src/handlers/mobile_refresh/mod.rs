mod credential;
mod failure;
mod metrics;

use backend_contracts::auth::{MobileState, RefreshOutcome};
use chrono::{DateTime, TimeDelta, Utc};
use sc_mobile::{Egress, MobileClient, MobileError, TokenSet};
use sqlx::PgPool;
use url::Url;
use uuid::Uuid;

use super::oauth_cooldowns::OAuthAppCooldowns;
use credential::{Claim, Credential};
use failure::Failure;

const FALLBACK_LIFETIME_SECONDS: i64 = 60 * 60;
const MAX_LIFETIME_SECONDS: i64 = 30 * 24 * 60 * 60;

pub struct MobileRefresh {
    pool: PgPool,
    cooldowns: OAuthAppCooldowns,
    proxy: Option<Url>,
}

pub struct MobileAnswer {
    pub outcome: RefreshOutcome,
    pub retry_after_sec: Option<i64>,
    pub state: Option<MobileState>,
}

type Standing = (RefreshOutcome, Option<i64>);

impl MobileRefresh {
    pub fn new(pool: PgPool, proxy: Option<Url>) -> Self {
        Self {
            cooldowns: OAuthAppCooldowns::new(pool.clone()),
            pool,
            proxy,
        }
    }

    pub async fn refresh(&self, connection_id: Uuid) -> Result<MobileAnswer, sqlx::Error> {
        let Some(credential) = Credential::load(&self.pool, connection_id).await? else {
            return Ok(MobileAnswer {
                outcome: RefreshOutcome::NotConnected,
                retry_after_sec: None,
                state: None,
            });
        };
        let (outcome, retry_after_sec) = match credential.standing() {
            Some(standing) => standing,
            None => self.attempt(connection_id, credential).await?,
        };
        let state = Credential::load(&self.pool, connection_id)
            .await?
            .map(|credential| credential.state());
        Ok(MobileAnswer {
            outcome,
            retry_after_sec,
            state,
        })
    }

    async fn attempt(
        &self,
        connection_id: Uuid,
        credential: Credential,
    ) -> Result<Standing, sqlx::Error> {
        let Some(app) = credential.app.clone() else {
            return Ok((RefreshOutcome::ReauthorizationRequired, None));
        };
        if let Some(seconds) = self
            .cooldowns
            .retry_after_seconds(credential.oauth_app_id)
            .await?
        {
            return Ok((RefreshOutcome::RateLimited, Some(seconds)));
        }
        let Some(claim) = credential.claim(&self.pool).await? else {
            return self.settled(connection_id).await;
        };
        let answer = match MobileClient::new(app, self.egress()) {
            Ok(client) => {
                client
                    .refresh(&credential.refresh_token, &credential.device())
                    .await
            }
            Err(error) => Err(error),
        };
        metrics::refresh(&answer);
        let recorded = match answer {
            Ok(tokens) => self.complete(&credential, &claim, tokens).await?,
            Err(error) => self.fail(&credential, &claim, &error).await?,
        };
        match recorded {
            Some(standing) => Ok(standing),
            None => self.settled(connection_id).await,
        }
    }

    async fn complete(
        &self,
        credential: &Credential,
        claim: &Claim,
        tokens: TokenSet,
    ) -> Result<Option<Standing>, sqlx::Error> {
        let stored = credential
            .complete(&self.pool, claim, &tokens, expiry(tokens.expires_in))
            .await?;
        Ok(stored.then_some((RefreshOutcome::Refreshed, None)))
    }

    async fn fail(
        &self,
        credential: &Credential,
        claim: &Claim,
        error: &MobileError,
    ) -> Result<Option<Standing>, sqlx::Error> {
        let failure = Failure::of(error);
        if failure == Failure::Relink {
            let recorded = credential.relink(&self.pool, claim, error).await?;
            return Ok(recorded.then_some((RefreshOutcome::ReauthorizationRequired, None)));
        }
        let mut seconds = failure.retry_seconds(credential.refresh_failure_count);
        if let Some(minimum) = failure.app_wide_seconds() {
            seconds = self
                .cooldowns
                .penalize(credential.oauth_app_id, minimum)
                .await?;
        }
        let seconds = i32::try_from(seconds.max(1)).unwrap_or(i32::MAX);
        let recorded = credential
            .retry(&self.pool, claim, failure.kind(), error, seconds)
            .await?;
        Ok(recorded.then_some((failure.outcome(), Some(i64::from(seconds)))))
    }

    async fn settled(&self, connection_id: Uuid) -> Result<Standing, sqlx::Error> {
        let standing = Credential::load(&self.pool, connection_id)
            .await?
            .map(|credential| {
                credential
                    .standing()
                    .unwrap_or((RefreshOutcome::RetryLater, Some(1)))
            });
        Ok(standing.unwrap_or((RefreshOutcome::NotConnected, None)))
    }

    fn egress(&self) -> Egress {
        match &self.proxy {
            Some(proxy) => Egress::ProxyThenDirect(proxy.clone()),
            None => Egress::Direct,
        }
    }
}

fn expiry(expires_in: Option<i64>) -> DateTime<Utc> {
    let seconds = expires_in
        .filter(|seconds| *seconds > 0)
        .unwrap_or(FALLBACK_LIFETIME_SECONDS)
        .min(MAX_LIFETIME_SECONDS);
    let now = Utc::now();
    TimeDelta::try_seconds(seconds)
        .and_then(|lifetime| now.checked_add_signed(lifetime))
        .unwrap_or(now)
}

#[cfg(test)]
mod tests;
