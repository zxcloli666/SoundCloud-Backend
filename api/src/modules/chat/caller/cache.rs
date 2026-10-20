use std::future::Future;
use std::time::Duration;

use chrono::{DateTime, Utc};
use mini_moka::sync::Cache;
use tokio::sync::Semaphore;
use uuid::Uuid;

use crate::modules::chat::caller::CallerBind;
use crate::modules::chat::model::DeviceRow;
use crate::modules::chat::{ChatError, ChatResult};

pub const CALLER_TTL: Duration = Duration::from_secs(600);
pub const SESSIONS_BYTES: u64 = 16 * 1024 * 1024;
pub const DEVICES_BYTES: u64 = 16 * 1024 * 1024;
pub const COLD_LOOKUPS: usize = 16;
pub const COLD_WAIT: Duration = Duration::from_millis(200);
pub const COLD_RETRY_MS: u64 = 1_000;

const ENTRY_OVERHEAD: u32 = 160;

#[derive(Clone, Debug)]
pub struct SessionEntry {
    pub user_id: String,
    pub bind: Option<CallerBind>,
}

#[derive(Clone, Debug)]
pub struct DeviceEntry {
    pub user_id: String,
    pub session_id: Uuid,
    pub auth_pub: Vec<u8>,
    pub revoked: bool,
    pub created_at: DateTime<Utc>,
    pub history_unlocked: bool,
}

impl From<&DeviceRow> for DeviceEntry {
    fn from(row: &DeviceRow) -> Self {
        Self {
            user_id: row.user_id.clone(),
            session_id: row.session_id,
            auth_pub: row.auth_pub.clone(),
            revoked: row.status != "active",
            created_at: row.created_at,
            history_unlocked: row.history_unlocked_at.is_some(),
        }
    }
}

pub struct Callers {
    sessions: Cache<Uuid, SessionEntry>,
    devices: Cache<Uuid, DeviceEntry>,
    cold: Semaphore,
}

impl Callers {
    pub fn new() -> Self {
        Self {
            sessions: Cache::builder()
                .weigher(|_, entry: &SessionEntry| weight(entry.user_id.len()))
                .max_capacity(SESSIONS_BYTES)
                .time_to_live(CALLER_TTL)
                .build(),
            devices: Cache::builder()
                .weigher(|_, entry: &DeviceEntry| {
                    weight(entry.user_id.len() + entry.auth_pub.len())
                })
                .max_capacity(DEVICES_BYTES)
                .time_to_live(CALLER_TTL)
                .build(),
            cold: Semaphore::new(COLD_LOOKUPS),
        }
    }

    pub fn session(&self, id: Uuid) -> Option<SessionEntry> {
        self.sessions.get(&id)
    }

    pub fn device(&self, id: Uuid) -> Option<DeviceEntry> {
        self.devices.get(&id)
    }

    pub fn remember_session(&self, id: Uuid, entry: SessionEntry) {
        self.sessions.insert(id, entry);
    }

    pub fn remember_device(&self, row: &DeviceRow) {
        self.devices.insert(row.id, DeviceEntry::from(row));
    }

    pub fn forget_session(&self, id: Uuid) {
        self.sessions.invalidate(&id);
    }

    pub fn forget_device(&self, id: Uuid) {
        self.devices.invalidate(&id);
    }

    pub async fn cold<T>(&self, lookup: impl Future<Output = ChatResult<T>>) -> ChatResult<T> {
        let busy = ChatError::Busy {
            retry_ms: COLD_RETRY_MS,
        };
        let Ok(Ok(_permit)) = tokio::time::timeout(COLD_WAIT, self.cold.acquire()).await else {
            return Err(busy);
        };
        lookup.await
    }
}

impl Default for Callers {
    fn default() -> Self {
        Self::new()
    }
}

fn weight(variable_bytes: usize) -> u32 {
    ENTRY_OVERHEAD.saturating_add(u32::try_from(variable_bytes).unwrap_or(u32::MAX))
}
