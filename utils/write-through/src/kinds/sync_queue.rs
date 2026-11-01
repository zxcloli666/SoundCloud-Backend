use std::fmt;

use chrono::{DateTime, Utc};
use serde::Serialize;
use uuid::Uuid;

use super::Kind;

#[derive(Debug, Serialize)]
pub struct SyncQueueRevive {
    pub user_ids: Vec<String>,
    pub target_urn: Option<String>,
    pub dropped_targets: Vec<String>,
    pub at: DateTime<Utc>,
}

impl Kind for SyncQueueRevive {
    const NAME: &'static str = "sync_queue.revive";
    type Output = Option<i64>;
}

#[derive(Debug, Serialize)]
pub struct SyncQueuePurge {
    pub min_retries: i32,
    pub at: DateTime<Utc>,
}

impl Kind for SyncQueuePurge {
    const NAME: &'static str = "sync_queue.purge";
    type Output = Option<i64>;
}

#[derive(Debug, Serialize)]
pub struct ConnectionDropOrphan {
    pub connection_id: Uuid,
}

impl Kind for ConnectionDropOrphan {
    const NAME: &'static str = "connection.drop_orphan";
    type Output = Option<bool>;
}

#[derive(Debug, Serialize)]
pub struct OauthAppDelete {
    pub app_id: Uuid,
}

impl Kind for OauthAppDelete {
    const NAME: &'static str = "oauth_app.delete";
    type Output = Option<bool>;
}

#[derive(Serialize)]
pub struct ConnectionLogin<'a> {
    pub connection_id: Uuid,
    pub soundcloud_user_id: &'a str,
    pub username: Option<&'a str>,
    pub oauth_app_id: Uuid,
    pub access_token: &'a str,
    pub refresh_token: &'a str,
    pub expires_at: DateTime<Utc>,
    pub scope: &'a str,
    pub login_v: i64,
    pub at: DateTime<Utc>,
}

impl fmt::Debug for ConnectionLogin<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ConnectionLogin")
            .field("connection_id", &self.connection_id)
            .field("soundcloud_user_id", &self.soundcloud_user_id)
            .field("oauth_app_id", &self.oauth_app_id)
            .field("login_v", &self.login_v)
            .field("at", &self.at)
            .finish_non_exhaustive()
    }
}

impl Kind for ConnectionLogin<'_> {
    const NAME: &'static str = "connection.login";
    type Output = bool;
}
