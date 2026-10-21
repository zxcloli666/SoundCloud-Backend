use std::collections::HashMap;
use std::time::Duration;

use mini_moka::sync::Cache;

use crate::modules::chat::{ChatResult, ChatState, presence};

const PREFS_TTL: Duration = Duration::from_secs(60);
const PREFS_KEPT: u64 = 200_000;
const SHARING_IDLE: Duration = Duration::from_secs(15 * 60);
const SHARING_TTL: Duration = Duration::from_secs(3_600);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Prefs {
    pub read_receipts: bool,
    pub typing: bool,
    pub presence_share: bool,
}

impl Default for Prefs {
    fn default() -> Self {
        Self {
            read_receipts: true,
            typing: true,
            presence_share: false,
        }
    }
}

pub struct PrefsCache {
    cache: Cache<String, Prefs>,
    sharing: Cache<String, bool>,
}

impl PrefsCache {
    pub fn new() -> Self {
        Self {
            cache: Cache::builder()
                .max_capacity(PREFS_KEPT)
                .time_to_live(PREFS_TTL)
                .build(),
            sharing: Cache::builder()
                .max_capacity(PREFS_KEPT)
                .time_to_idle(SHARING_IDLE)
                .time_to_live(SHARING_TTL)
                .build(),
        }
    }

    pub fn remember(&self, user_id: &str, prefs: Prefs) {
        self.cache.insert(user_id.to_owned(), prefs);
        self.sharing
            .insert(user_id.to_owned(), prefs.presence_share);
    }

    pub fn known(&self, user_id: &str) -> Option<Prefs> {
        self.cache.get(&user_id.to_owned())
    }

    pub fn sharing(&self, user_id: &str) -> Option<bool> {
        self.sharing.get(&user_id.to_owned())
    }

    #[cfg(test)]
    pub fn expire(&self, user_id: &str) {
        self.cache.invalidate(&user_id.to_owned());
    }
}

impl Default for PrefsCache {
    fn default() -> Self {
        Self::new()
    }
}

pub async fn changed(chat: &ChatState, user_id: &str, prefs: Prefs) -> ChatResult<()> {
    chat.hub.prefs.remember(user_id, prefs);
    if !prefs.presence_share {
        presence::forget(chat, user_id).await?;
    }
    Ok(())
}

pub async fn shares_presence(chat: &ChatState, user_id: &str) -> ChatResult<bool> {
    match chat.hub.prefs.sharing(user_id) {
        Some(sharing) => Ok(sharing),
        None => Ok(of(chat, user_id).await?.presence_share),
    }
}

pub async fn of(chat: &ChatState, user_id: &str) -> ChatResult<Prefs> {
    Ok(load(chat, &[user_id.to_owned()])
        .await?
        .remove(user_id)
        .unwrap_or_default())
}

pub async fn load(chat: &ChatState, users: &[String]) -> ChatResult<HashMap<String, Prefs>> {
    let mut found = HashMap::new();
    let mut missing = Vec::new();
    for user in users {
        match chat.hub.prefs.known(user) {
            Some(prefs) => {
                found.insert(user.clone(), prefs);
            }
            None => missing.push(user.clone()),
        }
    }
    missing.sort();
    missing.dedup();
    if missing.is_empty() {
        return Ok(found);
    }
    let rows = sqlx::query_file!("queries/chat/sync/prefs.sql", &missing)
        .fetch_all(&chat.pg)
        .await?;
    for row in rows {
        let prefs = Prefs {
            read_receipts: row.read_receipts,
            typing: row.typing,
            presence_share: row.presence_share,
        };
        chat.hub.prefs.remember(&row.user_id, prefs);
        found.insert(row.user_id, prefs);
    }
    for user in missing {
        found.entry(user).or_insert_with_key(|user| {
            chat.hub.prefs.remember(user, Prefs::default());
            Prefs::default()
        });
    }
    Ok(found)
}
