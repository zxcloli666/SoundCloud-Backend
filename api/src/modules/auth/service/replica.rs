use std::future::Future;
use std::time::Duration;

use axum::http::StatusCode;
use tokio::time::Instant;
use uuid::Uuid;

use crate::error::{AppError, AppResult};
use crate::modules::auth::model::{AuthSession, Session};

use super::AuthService;

const SESSION_WAIT: Duration = Duration::from_millis(1500);
const SESSION_POLL: Duration = Duration::from_millis(250);
const SESSION_RETRY_AFTER_SECONDS: i64 = 5;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Feed {
    Local,
    Streaming,
    Stalled,
}

impl AuthService {
    pub async fn find_session(&self, session_id: Uuid) -> AppResult<Option<Session>> {
        self.replicated(|| self.get_session(session_id)).await
    }

    pub async fn find_session_cached(&self, session_id: Uuid) -> AppResult<Option<Session>> {
        self.replicated(|| self.get_session_cached(session_id))
            .await
    }

    pub async fn find_auth_session(&self, session_id: Uuid) -> AppResult<Option<AuthSession>> {
        self.replicated(|| self.get_auth_session(session_id)).await
    }

    pub(super) async fn feed(&self) -> AppResult<Feed> {
        let row = sqlx::query_file!("queries/auth/service/replication_feed.sql")
            .fetch_one(&self.pool)
            .await?;
        Ok(if row.enabled == 0 {
            Feed::Local
        } else if row.stalled > 0 {
            Feed::Stalled
        } else {
            Feed::Streaming
        })
    }

    async fn replicated<T, F, Fut>(&self, lookup: F) -> AppResult<Option<T>>
    where
        F: Fn() -> Fut,
        Fut: Future<Output = AppResult<Option<T>>>,
    {
        if let Some(found) = lookup().await? {
            return Ok(Some(found));
        }
        match self.feed().await? {
            Feed::Local => Ok(None),
            Feed::Stalled => Err(session_not_replicated()),
            Feed::Streaming => {
                let deadline = Instant::now() + SESSION_WAIT;
                while Instant::now() < deadline {
                    tokio::time::sleep(SESSION_POLL).await;
                    if let Some(found) = lookup().await? {
                        return Ok(Some(found));
                    }
                }
                Ok(None)
            }
        }
    }
}

fn session_not_replicated() -> AppError {
    AppError::coded(
        StatusCode::SERVICE_UNAVAILABLE,
        "session_not_replicated",
        "This server has not received the session yet, retry shortly",
    )
    .with_retry_after(SESSION_RETRY_AFTER_SECONDS)
}
