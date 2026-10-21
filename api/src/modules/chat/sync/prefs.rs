use std::collections::HashMap;
use std::time::Duration;

use mini_moka::sync::Cache;

use crate::modules::chat::{ChatResult, ChatState};

const PREFS_TTL: Duration = Duration::from_secs(60);
const PREFS_KEPT: u64 = 200_000;

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
}

impl PrefsCache {
    pub fn new() -> Self {
        Self {
            cache: Cache::builder()
                .max_capacity(PREFS_KEPT)
                .time_to_live(PREFS_TTL)
                .build(),
        }
    }

    pub fn remember(&self, user_id: &str, prefs: Prefs) {
        self.cache.insert(user_id.to_owned(), prefs);
    }

    pub fn known(&self, user_id: &str) -> Option<Prefs> {
        self.cache.get(&user_id.to_owned())
    }
}

impl Default for PrefsCache {
    fn default() -> Self {
        Self::new()
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
