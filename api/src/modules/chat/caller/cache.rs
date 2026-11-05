use std::collections::HashSet;
use std::future::Future;
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use mini_moka::sync::Cache;
use tokio::sync::Semaphore;
use uuid::Uuid;

use crate::common::session::session_hash;
use crate::modules::chat::caller::CallerBind;
use crate::modules::chat::errors::BusySource;
use crate::modules::chat::held::{Held, STALE_FOR};
use crate::modules::chat::{ChatError, ChatResult};

pub const CALLER_TTL: Duration = Duration::from_secs(600);
pub const SESSIONS_BYTES: u64 = 16 * 1024 * 1024;
pub const COLD_LOOKUPS: usize = 16;
pub const UNKNOWN_TTL: Duration = Duration::from_secs(3);
pub const UNKNOWN_MAX: u64 = 16_384;
pub const COLD_WAIT: Duration = Duration::from_millis(200);
pub const REFRESH_WAIT: Duration = Duration::ZERO;
pub const USED_MAX: usize = 200_000;

const ENTRY_OVERHEAD: u32 = 160;
const COLD_TOTAL: &str = "chat_auth_cold_total";
const STALE_TOTAL: &str = "chat_auth_stale_total";
const USED_FULL_TOTAL: &str = "chat_session_check_untracked_total";

#[derive(Clone, Debug)]
pub struct SessionEntry {
    pub user_id: String,
    pub bind: Option<CallerBind>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cold {
    Session,
    Device,
    Alive,
}

impl Cold {
    fn label(self) -> &'static str {
        match self {
            Cold::Session => "session",
            Cold::Device => "device",
            Cold::Alive => "alive",
        }
    }

    fn busy(self) -> ChatError {
        ChatError::busy(match self {
            Cold::Session | Cold::Alive => BusySource::ColdSession,
            Cold::Device => BusySource::ColdDevice,
        })
    }

    pub fn count_stale(self) {
        metrics::counter!(STALE_TOTAL, "kind" => self.label()).increment(1);
    }
}

pub struct Callers {
    sessions: Cache<Vec<u8>, Held<SessionEntry>>,
    unknown: Cache<Uuid, ()>,
    used: Mutex<HashSet<Vec<u8>>>,
    cold: Semaphore,
    fresh_for: Duration,
    stale_for: Duration,
}

impl Callers {
    pub fn new() -> Self {
        Self::with_freshness(CALLER_TTL)
    }

    pub fn with_freshness(fresh_for: Duration) -> Self {
        Self::with_ages(fresh_for, STALE_FOR)
    }

    pub fn with_ages(fresh_for: Duration, stale_for: Duration) -> Self {
        Self {
            sessions: Cache::builder()
                .weigher(|_, held: &Held<SessionEntry>| weight(held.value.user_id.len()))
                .max_capacity(SESSIONS_BYTES)
                .time_to_live(CALLER_TTL)
                .build(),
            unknown: Cache::builder()
                .max_capacity(UNKNOWN_MAX)
                .time_to_live(UNKNOWN_TTL)
                .build(),
            used: Mutex::new(HashSet::new()),
            cold: Semaphore::new(COLD_LOOKUPS),
            fresh_for,
            stale_for,
        }
    }

    pub fn bytes(&self) -> u64 {
        self.sessions.weighted_size()
    }

    pub fn session(&self, id: Uuid) -> Option<Held<SessionEntry>> {
        let hash = session_hash(id);
        let held = self.sessions.get(&hash)?;
        self.mark_used([hash]);
        Some(held)
    }

    pub fn take_used(&self) -> Vec<Vec<u8>> {
        let mut used = self.used.lock().unwrap_or_else(PoisonError::into_inner);
        used.drain().collect()
    }

    pub fn mark_used(&self, hashes: impl IntoIterator<Item = Vec<u8>>) {
        let mut used = self.used.lock().unwrap_or_else(PoisonError::into_inner);
        for hash in hashes {
            if used.len() >= USED_MAX {
                metrics::counter!(USED_FULL_TOTAL).increment(1);
                return;
            }
            used.insert(hash);
        }
    }

    pub fn evict_hash(&self, session_hash: &[u8]) -> Option<SessionEntry> {
        let key = session_hash.to_vec();
        let held = self.sessions.get(&key)?;
        self.sessions.invalidate(&key);
        Some(held.value)
    }

    pub fn remember_session(&self, id: Uuid, entry: SessionEntry) {
        self.sessions.insert(
            session_hash(id),
            Held::new(entry, self.fresh_for, self.stale_for),
        );
    }

    pub fn is_unknown(&self, id: Uuid) -> bool {
        self.unknown.contains_key(&id)
    }

    pub fn remember_unknown(&self, id: Uuid) {
        self.unknown.insert(id, ());
    }

    pub fn forget_session(&self, id: Uuid) {
        self.forget_session_hash(&session_hash(id));
    }

    pub fn forget_session_hash(&self, session_hash: &[u8]) {
        self.sessions.invalidate(&session_hash.to_vec());
    }

    pub async fn cold<T>(
        &self,
        kind: Cold,
        wait: Duration,
        lookup: impl Future<Output = ChatResult<T>>,
    ) -> ChatResult<T> {
        metrics::counter!(COLD_TOTAL, "kind" => kind.label()).increment(1);
        let Ok(Ok(_permit)) = tokio::time::timeout(wait, self.cold.acquire()).await else {
            return Err(kind.busy());
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

#[cfg(test)]
#[path = "../tests/caller_cache_tests.rs"]
mod tests;
