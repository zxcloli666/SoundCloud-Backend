use std::future::Future;
use std::time::Duration;

use mini_moka::sync::Cache;
use tokio::sync::Semaphore;
use uuid::Uuid;

use crate::modules::chat::caller::CallerBind;
use crate::modules::chat::{ChatError, ChatResult};

pub const CALLER_TTL: Duration = Duration::from_secs(600);
pub const SESSIONS_BYTES: u64 = 16 * 1024 * 1024;
pub const COLD_LOOKUPS: usize = 16;
pub const COLD_WAIT: Duration = Duration::from_millis(200);

const ENTRY_OVERHEAD: u32 = 160;

#[derive(Clone, Debug)]
pub struct SessionEntry {
    pub user_id: String,
    pub bind: Option<CallerBind>,
}

pub struct Callers {
    sessions: Cache<Uuid, SessionEntry>,
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
            cold: Semaphore::new(COLD_LOOKUPS),
        }
    }

    pub fn bytes(&self) -> u64 {
        self.sessions.weighted_size()
    }

    pub fn session(&self, id: Uuid) -> Option<SessionEntry> {
        self.sessions.get(&id)
    }

    pub fn remember_session(&self, id: Uuid, entry: SessionEntry) {
        self.sessions.insert(id, entry);
    }

    pub fn forget_session(&self, id: Uuid) {
        self.sessions.invalidate(&id);
    }

    pub async fn cold<T>(&self, lookup: impl Future<Output = ChatResult<T>>) -> ChatResult<T> {
        metrics::counter!("chat_auth_cold_total").increment(1);
        let Ok(Ok(_permit)) = tokio::time::timeout(COLD_WAIT, self.cold.acquire()).await else {
            return Err(ChatError::busy());
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
