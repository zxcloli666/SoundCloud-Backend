use super::playlist_observe::{PlaylistReadClient, PlaylistReadError};
use super::sync_queue::{ConnectionError, ConnectionManager, TokenRefreshClient};
use crate::config::JobsConfig;
use crate::queue::{JobError, JobResult};
use serde_json::Value;
use sqlx::PgPool;
use std::time::Duration;

pub(super) struct CatalogRemote {
    pool: PgPool,
    owner: PlaylistReadClient,
    connections: ConnectionManager,
    tokens: TokenRefreshClient,
}

impl CatalogRemote {
    pub(super) fn new(pool: PgPool, config: &JobsConfig) -> Result<Self, crate::ClientBuildError> {
        Ok(Self {
            connections: ConnectionManager::new(pool.clone()),
            pool,
            owner: PlaylistReadClient::new(&config.sync_queue)?,
            tokens: TokenRefreshClient::new(&config.oauth)?,
        })
    }
    pub(super) async fn public_get(&self, path: &str) -> JobResult<Value> {
        let candidates = sqlx::query_file!(
            "../api/queries/oauth_apps/token_service/reload_snapshot.sql",
            chrono::Utc::now() + chrono::Duration::seconds(30)
        )
        .fetch_all(&self.pool)
        .await
        .map_err(JobError::retryable)?;
        let mut failure = postpone(
            60,
            anyhow::anyhow!("Public SoundCloud tokens are not ready"),
        );
        for candidate in candidates.into_iter().take(8) {
            if let Some(seconds) = self
                .connections
                .app_retry_after_seconds(candidate.oauth_app_id)
                .await
                .map_err(connection_error)?
            {
                failure = postpone(
                    seconds,
                    anyhow::anyhow!("Public SoundCloud application is cooling down"),
                );
                continue;
            }
            match self.owner.get_path(path, &candidate.access_token).await {
                Ok(response) => return Ok(response.value),
                Err(error) if error.is_unauthorized() => {
                    sqlx::query_file!(
                        "../api/queries/oauth_apps/token_service/reject_generation.sql",
                        candidate.oauth_app_id,
                        candidate.generation
                    )
                    .fetch_optional(&self.pool)
                    .await
                    .map_err(JobError::retryable)?;
                    failure = postpone(60, error);
                }
                Err(error) => {
                    let missing = matches!(&error, PlaylistReadError::Api { status, .. } if *status == wreq::StatusCode::NOT_FOUND);
                    let classified = self.owner_error(error, Some(candidate.oauth_app_id)).await;
                    if missing {
                        return Err(classified);
                    }
                    failure = classified;
                }
            }
        }
        Err(failure)
    }

    pub(super) async fn owner_get(&self, owner: &str, path: &str) -> JobResult<Value> {
        let token = self
            .connections
            .access_token(&self.tokens, owner)
            .await
            .map_err(connection_error)?;
        if let Some(app_id) = token.oauth_app_id
            && let Some(seconds) = self
                .connections
                .app_retry_after_seconds(app_id)
                .await
                .map_err(connection_error)?
        {
            return Err(postpone(
                seconds,
                anyhow::anyhow!("SoundCloud application is cooling down"),
            ));
        }
        match self.owner.get_path(path, &token.value).await {
            Ok(response) => Ok(response.value),
            Err(error) if error.is_unauthorized() => {
                let refreshed = self
                    .connections
                    .refresh_rejected_token(&self.tokens, owner, &token.value)
                    .await
                    .map_err(connection_error)?;
                match self.owner.get_path(path, &refreshed.value).await {
                    Ok(response) => Ok(response.value),
                    Err(error) => {
                        if error.is_unauthorized() {
                            self.connections
                                .reject_for_later(owner, &refreshed.value)
                                .await
                                .map_err(connection_error)?;
                        }
                        Err(self.owner_error(error, refreshed.oauth_app_id).await)
                    }
                }
            }
            Err(error) => Err(self.owner_error(error, token.oauth_app_id).await),
        }
    }

    async fn owner_error(&self, error: PlaylistReadError, app_id: Option<uuid::Uuid>) -> JobError {
        let cooldown = match &error {
            PlaylistReadError::Api {
                status,
                retry_after_seconds,
                ..
            } if *status == wreq::StatusCode::TOO_MANY_REQUESTS => {
                Some(retry_after_seconds.unwrap_or(300))
            }
            PlaylistReadError::Api { status, .. } if *status == wreq::StatusCode::FORBIDDEN => {
                Some(1800)
            }
            PlaylistReadError::Api { status, .. } if status.is_server_error() => Some(30),
            _ => None,
        };
        if let (Some(app_id), Some(seconds)) = (app_id, cooldown) {
            match self.connections.penalize_app(app_id, seconds).await {
                Ok(seconds) => return postpone(seconds, error),
                Err(error) => return connection_error(error),
            }
        }
        owner_error(error)
    }
}

pub(super) fn public_error(error: sc_transport::ScError) -> JobError {
    match &error {
        sc_transport::ScError::Api { status: 404, .. } => JobError::permanent(error),
        sc_transport::ScError::Api {
            status: 429,
            retry_after_sec,
            ..
        } => postpone(retry_after_sec.unwrap_or(300), error),
        sc_transport::ScError::Api { status: 403, .. } => postpone(1800, error),
        _ => JobError::retryable(error),
    }
}

pub(super) fn owner_error(error: PlaylistReadError) -> JobError {
    match &error {
        PlaylistReadError::Api { status, .. } if *status == wreq::StatusCode::NOT_FOUND => {
            JobError::permanent(error)
        }
        PlaylistReadError::Api {
            status,
            retry_after_seconds,
            ..
        } if *status == wreq::StatusCode::TOO_MANY_REQUESTS => {
            postpone(retry_after_seconds.unwrap_or(300), error)
        }
        PlaylistReadError::Api { status, .. } if *status == wreq::StatusCode::FORBIDDEN => {
            postpone(1800, error)
        }
        PlaylistReadError::Api { status, .. } if *status == wreq::StatusCode::UNAUTHORIZED => {
            postpone(300, error)
        }
        _ => JobError::retryable(error),
    }
}

pub(super) fn connection_error(error: ConnectionError) -> JobError {
    match &error {
        ConnectionError::ReauthorizationRequired => postpone(900, error),
        ConnectionError::RefreshInProgress {
            retry_after_seconds,
        }
        | ConnectionError::RateLimited {
            retry_after_seconds,
        }
        | ConnectionError::TemporarilyUnavailable {
            retry_after_seconds,
        } => postpone(*retry_after_seconds, error),
        ConnectionError::Database(_) => JobError::retryable(error),
    }
}

pub(super) fn postpone(seconds: i64, error: impl Into<anyhow::Error>) -> JobError {
    JobError::postponed(Duration::from_secs(seconds.clamp(1, 86400) as u64), error)
}
