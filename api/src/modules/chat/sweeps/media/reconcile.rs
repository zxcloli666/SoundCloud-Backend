pub mod cursor;
pub mod repair;

use std::collections::HashSet;
use std::time::Duration;

use chrono::{DateTime, Utc};
use uuid::Uuid;

pub use cursor::{Cursor, Stage};

use crate::modules::chat::s3::Listed;
use crate::modules::chat::sweeps::budget::Budget;
use crate::modules::chat::sweeps::media::{buckets_distinct, recount};
use crate::modules::chat::{ChatError, ChatResult, ChatState};

pub const PAGE: usize = 1_000;
pub const PAGES_PER_RUN: u32 = 60;
pub const ORPHAN_AGE: Duration = Duration::from_secs(24 * 3_600);
pub const LOST_PER_STORE_MAX: i64 = 1_000;
pub const ORPHANS_PER_STORE_MAX: i64 = 10_000;

pub async fn run(chat: &ChatState, now: DateTime<Utc>, budget: &mut Budget) -> ChatResult<u64> {
    if !buckets_distinct(&chat.config.media) {
        metrics::counter!("chat_media_reconcile_refused_total").increment(1);
        return Ok(0);
    }
    let stores = stores(chat);
    let Some(mut cursor) = Cursor::due(cursor::load(chat).await?, &stores, now) else {
        return Ok(0);
    };
    cursor.skip_unknown_store(&stores);
    let mut done = 0;
    while cursor.stage != Stage::Done && budget.spend(chat) {
        done += match cursor.stage {
            Stage::List => list_page(chat, now, &stores, &mut cursor).await?,
            Stage::Tail => tail_page(chat, &stores, &mut cursor).await?,
            Stage::Recount => recount_page(chat, &mut cursor).await?,
            Stage::Done => 0,
        };
        cursor::save(chat, &cursor).await?;
    }
    Ok(done)
}

pub fn over(cap: i64, so_far: i64, more: usize) -> bool {
    so_far.saturating_add(i64::try_from(more).unwrap_or(i64::MAX)) > cap
}

pub fn ours(key: &str) -> bool {
    key.strip_prefix("m/")
        .is_some_and(|rest| rest.len() == 32 && rest.bytes().all(|byte| byte.is_ascii_hexdigit()))
}

fn stores(chat: &ChatState) -> Vec<u16> {
    let mut ids: Vec<u16> = chat
        .config
        .media
        .stores
        .iter()
        .map(|store| store.id)
        .collect();
    ids.sort_unstable();
    ids
}

fn refuse(stores: &[u16], cursor: &mut Cursor, why: &'static str) -> u64 {
    tracing::error!(
        store = cursor.store,
        why,
        "chat media reconcile skips a store"
    );
    metrics::counter!("chat_media_reconcile_refused_total").increment(1);
    cursor.next_store(stores);
    0
}

async fn list_page(
    chat: &ChatState,
    now: DateTime<Utc>,
    stores: &[u16],
    cursor: &mut Cursor,
) -> ChatResult<u64> {
    let after = (!cursor.after.is_empty()).then_some(cursor.after.as_str());
    let page = chat.s3.list(cursor.store, after, PAGE).await?;
    let Some(last) = page.last().map(|listed| listed.key.clone()) else {
        if after.is_none() && repair::has_live_rows(chat, cursor.store).await? {
            return Ok(refuse(
                stores,
                cursor,
                "the store lists nothing while rows live there",
            ));
        }
        cursor.stage = Stage::Tail;
        return Ok(0);
    };
    let keys: Vec<String> = page.iter().map(|listed| listed.key.clone()).collect();
    let rows = sqlx::query_file!(
        "queries/chat/sweeps/media_reconcile_page.sql",
        store_id(cursor.store)?,
        &keys,
        cursor.after,
        last
    )
    .fetch_all(&chat.pg)
    .await?;
    let known: HashSet<&str> = rows
        .iter()
        .filter(|row| row.listed)
        .map(|row| row.object_key.as_str())
        .collect();
    let orphans: Vec<&Listed> = page
        .iter()
        .filter(|listed| {
            !known.contains(listed.key.as_str()) && ours(&listed.key) && aged(listed, now)
        })
        .collect();
    if over(ORPHANS_PER_STORE_MAX, cursor.orphans, orphans.len()) {
        return Ok(refuse(stores, cursor, "too many orphans for one night"));
    }
    let unlisted: Vec<(String, Uuid)> = rows
        .iter()
        .filter(|row| !row.listed)
        .map(|row| (row.object_key.clone(), row.root))
        .collect();
    let missing = repair::missing(chat, cursor.store, unlisted).await?;
    if over(LOST_PER_STORE_MAX, cursor.lost, missing.len()) {
        return Ok(refuse(
            stores,
            cursor,
            "too many missing objects for one night",
        ));
    }
    let deleted = repair::delete_orphans(chat, cursor.store, &orphans).await?;
    let lost = repair::mark_lost(chat, cursor.store, missing).await?;
    cursor.orphans += i64::try_from(deleted).unwrap_or(i64::MAX);
    cursor.lost += i64::try_from(lost).unwrap_or(i64::MAX);
    cursor.after = last;
    if page.len() < PAGE {
        cursor.stage = Stage::Tail;
    }
    Ok(deleted + lost)
}

async fn tail_page(chat: &ChatState, stores: &[u16], cursor: &mut Cursor) -> ChatResult<u64> {
    let rows = sqlx::query_file!(
        "queries/chat/sweeps/media_reconcile_tail.sql",
        store_id(cursor.store)?,
        cursor.after,
        PAGE as i64
    )
    .fetch_all(&chat.pg)
    .await?;
    let full = rows.len() == PAGE;
    let last = rows.last().map(|row| row.object_key.clone());
    let unlisted = rows
        .into_iter()
        .map(|row| (row.object_key, row.root))
        .collect();
    let missing = repair::missing(chat, cursor.store, unlisted).await?;
    if over(LOST_PER_STORE_MAX, cursor.lost, missing.len()) {
        return Ok(refuse(
            stores,
            cursor,
            "too many missing objects for one night",
        ));
    }
    let lost = repair::mark_lost(chat, cursor.store, missing).await?;
    cursor.lost += i64::try_from(lost).unwrap_or(i64::MAX);
    match last {
        Some(last) if full => cursor.after = last,
        _ => cursor.next_store(stores),
    }
    Ok(lost)
}

async fn recount_page(chat: &ChatState, cursor: &mut Cursor) -> ChatResult<u64> {
    let page = recount::page(chat, &cursor.after).await?;
    cursor.drift += page.drift;
    match page.last {
        Some(last) if page.full => cursor.after = last,
        _ => {
            cursor.stage = Stage::Done;
            cursor.after.clear();
            metrics::gauge!("chat_media_quota_drift_bytes").set(cursor.drift as f64);
        }
    }
    Ok(page.users)
}

fn aged(listed: &Listed, now: DateTime<Utc>) -> bool {
    now.signed_duration_since(listed.modified)
        .to_std()
        .is_ok_and(|age| age >= ORPHAN_AGE)
}

pub fn store_id(store: u16) -> ChatResult<i16> {
    i16::try_from(store).map_err(|_| ChatError::Internal)
}
