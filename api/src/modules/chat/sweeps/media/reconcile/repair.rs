use std::collections::BTreeSet;

use futures::{StreamExt, TryStreamExt, stream};
use uuid::Uuid;

use crate::modules::chat::media::ledger;
use crate::modules::chat::s3::Listed;
use crate::modules::chat::sweeps::media::reconcile::store_id;
use crate::modules::chat::{ChatResult, ChatState};

pub const PARALLEL: usize = 16;

pub async fn has_live_rows(chat: &ChatState, store: u16) -> ChatResult<bool> {
    let rows = sqlx::query_file!(
        "queries/chat/sweeps/media_reconcile_tail.sql",
        store_id(store)?,
        "",
        1
    )
    .fetch_all(&chat.pg)
    .await?;
    Ok(!rows.is_empty())
}

pub async fn delete_orphans(chat: &ChatState, store: u16, orphans: &[&Listed]) -> ChatResult<u64> {
    let keys: Vec<String> = orphans.iter().map(|listed| listed.key.clone()).collect();
    stream::iter(keys)
        .map(|key| delete_key(chat, store, key))
        .buffer_unordered(PARALLEL)
        .try_collect::<Vec<()>>()
        .await?;
    if !orphans.is_empty() {
        metrics::counter!("chat_media_orphans_deleted_total").increment(orphans.len() as u64);
    }
    Ok(orphans.len() as u64)
}

pub async fn missing(
    chat: &ChatState,
    store: u16,
    unlisted: Vec<(String, Uuid)>,
) -> ChatResult<Vec<Uuid>> {
    let heads: Vec<Option<Uuid>> = stream::iter(unlisted)
        .map(|(key, root)| gone(chat, store, key, root))
        .buffer_unordered(PARALLEL)
        .try_collect()
        .await?;
    let roots: BTreeSet<Uuid> = heads.into_iter().flatten().collect();
    Ok(roots.into_iter().collect())
}

async fn delete_key(chat: &ChatState, store: u16, key: String) -> ChatResult<()> {
    chat.s3.delete(store, &key).await
}

async fn gone(chat: &ChatState, store: u16, key: String, root: Uuid) -> ChatResult<Option<Uuid>> {
    Ok(chat.s3.head(store, &key).await?.is_none().then_some(root))
}

pub async fn mark_lost(chat: &ChatState, store: u16, missing: Vec<Uuid>) -> ChatResult<u64> {
    if missing.is_empty() {
        return Ok(0);
    }
    let mut tx = chat.pg.begin().await?;
    ledger::lock(&mut tx, &missing).await?;
    ledger::lock_owners(&mut tx, &missing).await?;
    let lost = sqlx::query_file_scalar!(
        "queries/chat/sweeps/media_mark_lost.sql",
        &missing,
        store_id(store)?
    )
    .fetch_all(&mut *tx)
    .await?;
    let count = lost.len() as u64;
    ledger::settle(&mut tx, &lost).await?;
    tx.commit().await?;
    if count > 0 {
        metrics::counter!("chat_media_lost_total").increment(count);
    }
    Ok(count)
}
