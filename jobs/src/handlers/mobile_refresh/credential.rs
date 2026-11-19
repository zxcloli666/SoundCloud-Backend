use backend_contracts::auth::{MobileState, RefreshOutcome};
use chrono::{DateTime, TimeDelta, Utc};
use redact::Secret;
use sc_mobile::{MobileApp, MobileError, TokenSet};
use sqlx::PgPool;
use uuid::Uuid;

use super::Standing;

const LEASE_SECONDS: i32 = 60;
const REFRESH_DUE_BUFFER: TimeDelta = TimeDelta::minutes(5);
const RELINK_REQUIRED: &str = "relink_required";
const RATE_LIMITED: &str = "rate_limited";
const MESSAGE_LIMIT: usize = 200;

pub(super) struct Credential {
    soundcloud_user_id: String,
    pub(super) oauth_app_id: Uuid,
    device_id: Uuid,
    access_token: Secret<String>,
    pub(super) refresh_token: Secret<String>,
    expires_at: DateTime<Utc>,
    status: String,
    refresh_generation: i64,
    pub(super) refresh_failure_count: i32,
    refresh_lease_id: Option<Uuid>,
    refresh_lease_expires_at: Option<DateTime<Utc>>,
    retry_at: Option<DateTime<Utc>>,
    last_refresh_error_kind: Option<String>,
    pub(super) app: Option<MobileApp>,
}

pub(super) struct Claim {
    lease_id: Uuid,
    generation: i64,
}

impl Credential {
    pub(super) async fn load(
        pool: &PgPool,
        connection_id: Uuid,
    ) -> Result<Option<Self>, sqlx::Error> {
        let row = sqlx::query_file!("queries/mobile_refresh/load.sql", connection_id)
            .fetch_optional(pool)
            .await?;
        let Some(row) = row else {
            return Ok(None);
        };
        let app = match (row.app_usable, row.app_id, row.app_version, row.user_agent) {
            (true, Some(app_id), Some(app_version), Some(user_agent)) => Some(MobileApp {
                client_id: row.client_id,
                client_secret: Secret::new(row.client_secret),
                app_id,
                app_version,
                user_agent,
            }),
            _ => None,
        };
        Ok(Some(Self {
            soundcloud_user_id: row.soundcloud_user_id,
            oauth_app_id: row.oauth_app_id,
            device_id: row.device_id,
            access_token: Secret::new(row.access_token),
            refresh_token: Secret::new(row.refresh_token),
            expires_at: row.expires_at,
            status: row.status,
            refresh_generation: row.refresh_generation,
            refresh_failure_count: row.refresh_failure_count,
            refresh_lease_id: row.refresh_lease_id,
            refresh_lease_expires_at: row.refresh_lease_expires_at,
            retry_at: row.retry_at,
            last_refresh_error_kind: row.last_refresh_error_kind,
            app,
        }))
    }

    pub(super) fn standing(&self) -> Option<Standing> {
        let now = Utc::now();
        if self.status == RELINK_REQUIRED {
            return Some((RefreshOutcome::ReauthorizationRequired, None));
        }
        if self.expires_at > now + REFRESH_DUE_BUFFER {
            return Some((RefreshOutcome::AlreadyFresh, None));
        }
        if let Some(seconds) = self.lease_seconds(now) {
            return Some((RefreshOutcome::InProgress, Some(seconds)));
        }
        let seconds = seconds_until(self.retry_at, now)?;
        let outcome = if self.last_refresh_error_kind.as_deref() == Some(RATE_LIMITED) {
            RefreshOutcome::RateLimited
        } else {
            RefreshOutcome::RetryLater
        };
        Some((outcome, Some(seconds)))
    }

    pub(super) fn state(&self) -> MobileState {
        let lease_expires_at = self.refresh_lease_id.and(self.refresh_lease_expires_at);
        MobileState {
            status: self.status.clone(),
            expires_at_unix_ms: self.expires_at.timestamp_millis(),
            refresh_lease_expires_at_unix_ms: lease_expires_at.map(|at| at.timestamp_millis()),
            retry_at_unix_ms: self.retry_at.map(|at| at.timestamp_millis()),
            last_refresh_error_kind: self.last_refresh_error_kind.clone(),
        }
    }

    pub(super) fn device(&self) -> String {
        self.device_id.simple().to_string()
    }

    pub(super) async fn claim(&self, pool: &PgPool) -> Result<Option<Claim>, sqlx::Error> {
        let lease_id = Uuid::new_v4();
        let generation = sqlx::query_file_scalar!(
            "queries/mobile_refresh/claim.sql",
            self.soundcloud_user_id,
            self.refresh_generation,
            self.access_token.expose(),
            lease_id,
            LEASE_SECONDS
        )
        .fetch_optional(pool)
        .await?;
        Ok(generation.map(|generation| Claim {
            lease_id,
            generation,
        }))
    }

    pub(super) async fn complete(
        &self,
        pool: &PgPool,
        claim: &Claim,
        tokens: &TokenSet,
        expires_at: DateTime<Utc>,
    ) -> Result<bool, sqlx::Error> {
        let refresh_token = tokens
            .refresh_token
            .as_ref()
            .map(|token| token.expose().as_str());
        let stored = sqlx::query_file_scalar!(
            "queries/mobile_refresh/complete.sql",
            self.soundcloud_user_id,
            claim.lease_id,
            claim.generation,
            tokens.access_token.expose(),
            refresh_token,
            expires_at,
            tokens.scope.as_deref()
        )
        .fetch_optional(pool)
        .await?;
        Ok(stored.is_some())
    }

    pub(super) async fn retry(
        &self,
        pool: &PgPool,
        claim: &Claim,
        kind: &str,
        error: &MobileError,
        seconds: i32,
    ) -> Result<bool, sqlx::Error> {
        let recorded = sqlx::query_file!(
            "queries/mobile_refresh/fail_retryable.sql",
            self.soundcloud_user_id,
            claim.lease_id,
            claim.generation,
            kind,
            bounded(error),
            seconds
        )
        .execute(pool)
        .await?;
        Ok(recorded.rows_affected() > 0)
    }

    pub(super) async fn relink(
        &self,
        pool: &PgPool,
        claim: &Claim,
        error: &MobileError,
    ) -> Result<bool, sqlx::Error> {
        let recorded = sqlx::query_file!(
            "queries/mobile_refresh/fail_relink.sql",
            self.soundcloud_user_id,
            claim.lease_id,
            claim.generation,
            bounded(error)
        )
        .execute(pool)
        .await?;
        Ok(recorded.rows_affected() > 0)
    }

    fn lease_seconds(&self, now: DateTime<Utc>) -> Option<i64> {
        self.refresh_lease_id?;
        seconds_until(self.refresh_lease_expires_at, now)
    }
}

fn seconds_until(at: Option<DateTime<Utc>>, now: DateTime<Utc>) -> Option<i64> {
    let at = at?;
    (at > now).then(|| (at - now).num_seconds().max(1))
}

fn bounded(error: &MobileError) -> String {
    error.to_string().chars().take(MESSAGE_LIMIT).collect()
}
