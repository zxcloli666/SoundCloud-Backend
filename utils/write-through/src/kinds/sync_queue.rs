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
