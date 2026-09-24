mod connection;
pub(crate) mod login;
mod oauth;

use std::sync::Arc;

use sqlx::PgPool;
use uuid::Uuid;

use crate::error::AppResult;
use crate::modules::auth::health::AuthHealthService;
use crate::modules::auth::model::{AuthSession, Session, SoundCloudConnection};
use crate::modules::oauth_apps::OAuthAppsService;
use crate::sc::ScClient;

pub use connection::{RefreshAttempt, RefreshOutcome};

#[derive(Clone)]
pub struct AuthService {
    pool: PgPool,
    sc: ScClient,
    oauth_apps: Arc<OAuthAppsService>,
    health: Arc<AuthHealthService>,
}

impl AuthService {
    pub fn new(
        pool: PgPool,
        sc: ScClient,
        oauth_apps: Arc<OAuthAppsService>,
        health: Arc<AuthHealthService>,
    ) -> Arc<Self> {
        Arc::new(Self {
            pool,
            sc,
            oauth_apps,
            health,
        })
    }

    pub async fn get_session(&self, session_id: Uuid) -> AppResult<Option<Session>> {
        let session =
            sqlx::query_file_as!(Session, "queries/auth/service/get_session.sql", session_id)
                .fetch_optional(&self.pool)
                .await?;
        Ok(session)
    }

    pub async fn get_auth_session(&self, session_id: Uuid) -> AppResult<Option<AuthSession>> {
        let session = sqlx::query_file_as!(
            AuthSession,
            "queries/auth/service/get_auth_session.sql",
            session_id
        )
        .fetch_optional(&self.pool)
        .await?;
        Ok(session)
    }

    pub async fn get_connection_for_session(
        &self,
        session_id: Uuid,
    ) -> AppResult<Option<SoundCloudConnection>> {
        let connection = sqlx::query_file_as!(
            SoundCloudConnection,
            "queries/auth/service/get_connection_by_session.sql",
            session_id
        )
        .fetch_optional(&self.pool)
        .await?;
        Ok(connection)
    }

    async fn get_connection_by_id(
        &self,
        connection_id: Uuid,
    ) -> AppResult<Option<SoundCloudConnection>> {
        let connection = sqlx::query_file_as!(
            SoundCloudConnection,
            "queries/auth/service/get_connection_by_id.sql",
            connection_id
        )
        .fetch_optional(&self.pool)
        .await?;
        Ok(connection)
    }

    pub async fn logout(&self, session_id: Uuid) -> AppResult<()> {
        let mut transaction = self.pool.begin().await?;
        let deleted = sqlx::query_file!("queries/auth/service/delete_session.sql", session_id)
            .fetch_optional(&mut *transaction)
            .await?;
        if let Some(connection_id) = deleted.and_then(|row| row.soundcloud_connection_id) {
            sqlx::query_file!(
                "queries/auth/service/delete_orphan_connection.sql",
                connection_id
            )
            .execute(&mut *transaction)
            .await?;
        }
        transaction.commit().await?;
        Ok(())
    }
}
