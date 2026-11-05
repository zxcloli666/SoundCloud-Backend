use std::collections::{HashMap, HashSet};
use std::sync::Mutex;
use std::time::Instant;

use sqlx::PgPool;
use uuid::Uuid;

use crate::modules::chat::media::limits::STORE_FILL_TTL;
use crate::modules::chat::media::{MediaConfig, StoreConfig};
use crate::modules::chat::{ChatResult, ChatState};

#[derive(Default)]
pub struct Fills {
    known: Mutex<HashMap<u16, (Instant, u64)>>,
    refresh: tokio::sync::Mutex<()>,
}

impl Fills {
    fn known(&self, store: u16) -> Option<(Instant, u64)> {
        let known = self
            .known
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        known.get(&store).copied()
    }

    fn fresh(&self, store: u16) -> Option<u64> {
        self.known(store)
            .filter(|(at, _)| at.elapsed() < STORE_FILL_TTL)
            .map(|(_, bytes)| bytes)
    }

    fn remember(&self, store: u16, bytes: u64) {
        let mut known = self
            .known
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        known.insert(store, (Instant::now(), bytes));
    }
}

#[derive(Debug, Clone)]
pub struct Object {
    pub id: Uuid,
    pub key: String,
    pub bytes: u64,
}

pub fn new_key() -> String {
    format!("m/{:032x}", rand::random::<u128>())
}

pub fn get(config: &MediaConfig, id: u16) -> Option<&StoreConfig> {
    config.stores.iter().find(|store| store.id == id)
}

pub fn write(config: &MediaConfig) -> Option<&StoreConfig> {
    get(config, config.write?)
}

pub fn cap_bytes(store: &StoreConfig) -> u64 {
    store.cap_gb.saturating_mul(1 << 30)
}

pub async fn filled(chat: &ChatState, store: u16) -> ChatResult<u64> {
    let fills = &chat.fills;
    if let Some(bytes) = fills.fresh(store) {
        return Ok(bytes);
    }
    let _refreshing = match (fills.refresh.try_lock(), fills.known(store)) {
        (Ok(guard), _) => guard,
        (Err(_), Some((_, stale))) => return Ok(stale),
        (Err(_), None) => fills.refresh.lock().await,
    };
    if let Some(bytes) = fills.fresh(store) {
        return Ok(bytes);
    }
    let bytes = sqlx::query_file_scalar!(
        "queries/chat/media/store_fill.sql",
        i16::try_from(store).unwrap_or(i16::MAX)
    )
    .fetch_one(&chat.pg)
    .await?;
    let bytes = u64::try_from(bytes).unwrap_or(0);
    fills.remember(store, bytes);
    Ok(bytes)
}

pub async fn open_for_writes(chat: &ChatState) -> ChatResult<Option<u16>> {
    let Some(store) = write(&chat.config.media) else {
        return Ok(None);
    };
    if !chat.s3.serves(store.id) {
        return Ok(None);
    }
    let under_cap = filled(chat, store.id).await? < cap_bytes(store);
    Ok(under_cap.then_some(store.id))
}

pub fn missing(config: &MediaConfig, used: &[i16]) -> Vec<i16> {
    let known: HashSet<i16> = config
        .stores
        .iter()
        .filter_map(|store| i16::try_from(store.id).ok())
        .collect();
    used.iter()
        .copied()
        .filter(|id| !known.contains(id))
        .collect()
}

pub async fn verify_rows(pg: &PgPool, config: &MediaConfig) -> anyhow::Result<()> {
    let used = sqlx::query_file_scalar!("queries/chat/media/stores_in_use.sql")
        .fetch_all(pg)
        .await?;
    let absent = missing(config, &used);
    anyhow::ensure!(
        absent.is_empty(),
        "chat.media rows name stores {absent:?} that CHAT_STORES does not list"
    );
    if let Some(write) = config.write {
        anyhow::ensure!(
            get(config, write).is_some(),
            "CHAT_STORE_WRITE names store {write} that CHAT_STORES does not list"
        );
    }
    Ok(())
}

#[cfg(test)]
#[path = "tests/stores_tests.rs"]
mod tests;
