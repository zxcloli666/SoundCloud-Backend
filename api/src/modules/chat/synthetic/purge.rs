use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use serde::Serialize;
use tokio::time::Instant;
use uuid::Uuid;

use crate::modules::chat::sweeps::budget::Budget;
use crate::modules::chat::sweeps::purge;
use crate::modules::chat::synthetic::{USER_PREFIX, sessions};
use crate::modules::chat::{ChatResult, ChatState, media, rt_keys};

pub const BATCH_ROWS: i64 = 5_000;
pub const CONVERSATIONS_PER_PAGE: i64 = 256;
pub const PURGE_WINDOW: Duration = Duration::from_secs(20);
pub const SCAN_COUNT: u32 = 1_000;

#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
pub struct Purged {
    pub devices: u64,
    pub sessions: u64,
    pub marked: u64,
    pub purged: u64,
    pub memberships: u64,
    pub relations: u64,
    pub users: u64,
    pub redis_keys: u64,
    pub pending: i64,
}

pub async fn run(chat: &ChatState) -> ChatResult<Purged> {
    let deadline = Instant::now() + PURGE_WINDOW;
    let mut purged = Purged {
        devices: drop_devices(chat).await?,
        sessions: sessions::drop_all(chat).await?,
        ..Purged::default()
    };
    purged.marked = sqlx::query_file!("queries/chat/synthetic/mark_conversations.sql")
        .execute(&chat.pg)
        .await?
        .rows_affected();
    purged.purged = purge_marked(chat, deadline).await?;
    purged.memberships = drop_memberships(chat).await?;
    purged.relations = drop_relations(chat).await?;
    purged.pending = sqlx::query_file_scalar!("queries/chat/synthetic/pending.sql")
        .fetch_one(&chat.pg)
        .await?;
    if purged.pending == 0 {
        purged.users = sqlx::query_file!("queries/chat/synthetic/drop_users.sql")
            .execute(&chat.pg)
            .await?
            .rows_affected();
    }
    purged.redis_keys = forget_keys(chat).await?;
    Ok(purged)
}

async fn drop_devices(chat: &ChatState) -> ChatResult<u64> {
    let mut devices = 0;
    loop {
        let rows = sqlx::query_file!("queries/chat/synthetic/drop_devices.sql", BATCH_ROWS)
            .fetch_all(&chat.pg)
            .await?;
        let count = rows.len();
        let mut by_user: BTreeMap<String, Vec<Uuid>> = BTreeMap::new();
        for row in rows {
            by_user.entry(row.user_id).or_default().push(row.id);
        }
        for (user_id, devices) in &by_user {
            chat.devices.forget(user_id, devices);
        }
        devices += count as u64;
        if count == 0 {
            return Ok(devices);
        }
    }
}

async fn purge_marked(chat: &ChatState, deadline: Instant) -> ChatResult<u64> {
    let mut purged = 0;
    let mut budget = Budget::until(u32::MAX, deadline);
    while budget.left(chat) {
        let page = sqlx::query_file_scalar!(
            "queries/chat/synthetic/marked_conversations.sql",
            CONVERSATIONS_PER_PAGE
        )
        .fetch_all(&chat.pg)
        .await?;
        if page.is_empty() {
            break;
        }
        for conversation in page {
            if !budget.left(chat) {
                return Ok(purged);
            }
            if purge::conversation(chat, conversation, &mut budget).await? {
                purged += 1;
            }
        }
    }
    Ok(purged)
}

async fn drop_memberships(chat: &ChatState) -> ChatResult<u64> {
    let mut tx = chat.pg.begin().await?;
    let touched: Vec<Uuid> =
        sqlx::query_file_scalar!("queries/chat/synthetic/drop_memberships.sql")
            .fetch_all(&mut *tx)
            .await?;
    let conversations: BTreeSet<Uuid> = touched.iter().copied().collect();
    for conversation in &conversations {
        sqlx::query_file_scalar!("queries/chat/conversations/recount.sql", conversation)
            .fetch_optional(&mut *tx)
            .await?;
        media::hooks::queue_trim(&mut tx, *conversation).await?;
    }
    tx.commit().await?;
    for conversation in &conversations {
        chat.hot.forget(*conversation);
    }
    Ok(touched.len() as u64)
}

async fn drop_relations(chat: &ChatState) -> ChatResult<u64> {
    let blocks = sqlx::query_file!("queries/chat/synthetic/drop_blocks.sql")
        .execute(&chat.pg)
        .await?
        .rows_affected();
    let reports = sqlx::query_file!("queries/chat/synthetic/drop_reports.sql")
        .execute(&chat.pg)
        .await?
        .rows_affected();
    Ok(blocks + reports)
}

pub fn key_patterns() -> [String; 3] {
    let synthetic = format!("{USER_PREFIX}*");
    [
        rt_keys::version(&synthetic),
        rt_keys::deltas(&synthetic),
        rt_keys::limit("*", &synthetic),
    ]
}

async fn forget_keys(chat: &ChatState) -> ChatResult<u64> {
    let mut removed = 0;
    for pattern in key_patterns() {
        let mut cursor = 0u64;
        loop {
            let mut conn = chat.redis.get().await?;
            let (next, keys): (u64, Vec<String>) = redis::cmd("SCAN")
                .arg(cursor)
                .arg("MATCH")
                .arg(&pattern)
                .arg("COUNT")
                .arg(SCAN_COUNT)
                .query_async(&mut conn)
                .await?;
            if !keys.is_empty() {
                let gone: u64 = redis::cmd("DEL").arg(&keys).query_async(&mut conn).await?;
                removed += gone;
            }
            if next == 0 {
                break;
            }
            cursor = next;
        }
    }
    Ok(removed)
}
