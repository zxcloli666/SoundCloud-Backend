use std::time::Duration;

use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::config::OAuthAppBootstrap;

use super::super::model::{ClaimedApp, Reservation, Token};
use super::{BOOTSTRAP_LOCK, EGRESS_LOCK, OAuthRefreshRepository};

impl OAuthRefreshRepository {
    pub fn new(pool: sqlx::PgPool) -> Self {
        Self { pool }
    }

    pub async fn bootstrap_app(&self, app: &OAuthAppBootstrap) -> Result<Uuid, sqlx::Error> {
        let mut transaction = self.pool.begin().await?;
        sqlx::query("SELECT pg_advisory_xact_lock($1)")
            .bind(BOOTSTRAP_LOCK)
            .execute(&mut *transaction)
            .await?;
        let inserted = sqlx::query_file_scalar!(
            "queries/oauth_apps/bootstrap.sql",
            app.id(),
            &app.name,
            &app.client_id,
            app.client_secret.expose().as_str(),
            &app.redirect_uri
        )
        .fetch_one(&mut *transaction)
        .await?;
        transaction.commit().await?;
        Ok(inserted)
    }

    pub async fn claim_due(
        &self,
        limit: i64,
        lease_duration: Duration,
    ) -> Result<Vec<ClaimedApp>, sqlx::Error> {
        let lease_milliseconds = i64::try_from(lease_duration.as_millis())
            .map_err(|error| sqlx::Error::Protocol(error.to_string()))?;
        sqlx::query_file_as!(
            ClaimedApp,
            "queries/oauth_apps/claim_due.sql",
            limit,
            lease_milliseconds
        )
        .fetch_all(&self.pool)
        .await
    }

    pub async fn reserve_client_credentials(
        &self,
        claim: &ClaimedApp,
    ) -> Result<Reservation, sqlx::Error> {
        sqlx::query_file_as!(
            Reservation,
            "queries/oauth_apps/reserve_client_credentials.sql",
            EGRESS_LOCK,
            Uuid::now_v7(),
            claim.id,
            &claim.client_id
        )
        .fetch_one(&self.pool)
        .await
    }

    pub async fn release_reservation(&self, reservation_id: Uuid) -> Result<(), sqlx::Error> {
        sqlx::query_file!("queries/oauth_apps/release_reservation.sql", reservation_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn complete(&self, claim: &ClaimedApp, token: &Token) -> Result<bool, sqlx::Error> {
        let stored = sqlx::query_file_scalar!(
            "queries/oauth_apps/complete_refresh.sql",
            claim.id,
            claim.lease_id,
            &token.access_token,
            token.refresh_token.as_deref(),
            token.scope.as_deref(),
            token.expires_in
        )
        .fetch_optional(&self.pool)
        .await?;
        Ok(stored.is_some())
    }

    pub async fn retry(
        &self,
        claim: &ClaimedApp,
        retry_at: DateTime<Utc>,
        reason: &str,
    ) -> Result<bool, sqlx::Error> {
        let recorded = sqlx::query_file_scalar!(
            "queries/oauth_apps/retry_refresh.sql",
            claim.id,
            claim.lease_id,
            retry_at,
            reason
        )
        .fetch_optional(&self.pool)
        .await?;
        Ok(recorded.is_some())
    }
}
