use std::future::Future;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::http::StatusCode;
use tokio::time::Instant;
use uuid::{Uuid, Version};

use crate::error::{AppError, AppResult};
use crate::modules::auth::model::{AuthSession, Session};

use super::AuthService;

const SESSION_WAIT: Duration = Duration::from_millis(750);
const SESSION_POLL: Duration = Duration::from_millis(250);
const SESSION_IN_FLIGHT: Duration = Duration::from_secs(30 * 60);
pub const SESSION_RETRY_AFTER_SECONDS: i64 = 5;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Feed {
    Local,
    Streaming,
    Stalled,
}

impl AuthService {
    pub async fn find_session(&self, session_id: Uuid) -> AppResult<Option<Session>> {
        self.replicated(session_id, || self.get_session(session_id))
            .await
    }

    pub async fn find_session_cached(&self, session_id: Uuid) -> AppResult<Option<Session>> {
        self.replicated(session_id, || self.get_session_cached(session_id))
            .await
    }

    pub async fn find_auth_session(&self, session_id: Uuid) -> AppResult<Option<AuthSession>> {
        self.replicated(session_id, || self.get_auth_session(session_id))
            .await
    }

    pub(super) async fn feed(&self) -> AppResult<Feed> {
        let row = sqlx::query_file!("queries/auth/service/replication_feed.sql")
            .fetch_one(&self.pool)
            .await?;
        Ok(if row.enabled == 0 && !row.split {
            Feed::Local
        } else if row.enabled == 0 || row.stalled > 0 {
            Feed::Stalled
        } else {
            Feed::Streaming
        })
    }

    pub(crate) async fn replicated_now<T, F, Fut>(&self, lookup: F) -> AppResult<Option<T>>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = AppResult<Option<T>>>,
    {
        if let Some(found) = lookup().await? {
            return Ok(Some(found));
        }
        match self.feed().await? {
            Feed::Stalled => Err(session_not_replicated()),
            Feed::Local | Feed::Streaming => Ok(None),
        }
    }

    async fn replicated<T, F, Fut>(&self, session_id: Uuid, lookup: F) -> AppResult<Option<T>>
    where
        F: Fn() -> Fut,
        Fut: Future<Output = AppResult<Option<T>>>,
    {
        if let Some(found) = lookup().await? {
            return Ok(Some(found));
        }
        let feed = self.feed().await?;
        self.missed(feed, session_id, lookup).await
    }

    async fn missed<T, F, Fut>(
        &self,
        feed: Feed,
        session_id: Uuid,
        lookup: F,
    ) -> AppResult<Option<T>>
    where
        F: Fn() -> Fut,
        Fut: Future<Output = AppResult<Option<T>>>,
    {
        match feed {
            Feed::Local => Ok(dead()),
            Feed::Stalled => {
                crate::metrics::record_session_miss("stalled");
                Err(session_not_replicated())
            }
            Feed::Streaming => {
                if !in_flight(session_id, SystemTime::now()) || self.logged_out(session_id).await? {
                    return Ok(dead());
                }
                wait_for(lookup).await
            }
        }
    }

    async fn logged_out(&self, session_id: Uuid) -> AppResult<bool> {
        let logged_out =
            sqlx::query_file_scalar!("queries/auth/service/session_logged_out.sql", session_id)
                .fetch_one(&self.pool)
                .await?;
        Ok(logged_out)
    }
}

async fn wait_for<T, F, Fut>(lookup: F) -> AppResult<Option<T>>
where
    F: Fn() -> Fut,
    Fut: Future<Output = AppResult<Option<T>>>,
{
    let deadline = Instant::now() + SESSION_WAIT;
    while Instant::now() < deadline {
        tokio::time::sleep(SESSION_POLL).await;
        if let Some(found) = lookup().await? {
            crate::metrics::record_session_miss("found_after_wait");
            return Ok(Some(found));
        }
    }
    crate::metrics::record_session_miss("in_flight");
    Err(session_not_replicated())
}

fn dead<T>() -> Option<T> {
    crate::metrics::record_session_miss("dead");
    None
}

fn in_flight(session_id: Uuid, now: SystemTime) -> bool {
    if session_id.get_version() != Some(Version::SortRand) {
        return false;
    }
    let Some((seconds, nanos)) = session_id.get_timestamp().map(|minted| minted.to_unix()) else {
        return false;
    };
    let minted = UNIX_EPOCH + Duration::new(seconds, nanos);
    let age = now
        .duration_since(minted)
        .unwrap_or_else(|ahead| ahead.duration());
    age < SESSION_IN_FLIGHT
}

pub const NOT_REPLICATED: &str = "session_not_replicated";

fn session_not_replicated() -> AppError {
    AppError::coded(
        StatusCode::SERVICE_UNAVAILABLE,
        NOT_REPLICATED,
        "This server has not received the session yet, retry shortly",
    )
    .with_retry_after(SESSION_RETRY_AFTER_SECONDS)
}

#[cfg(test)]
#[path = "../session_miss_tests.rs"]
mod tests;
