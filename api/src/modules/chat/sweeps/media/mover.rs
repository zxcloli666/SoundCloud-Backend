use std::sync::LazyLock;

use futures::{StreamExt, stream};
use redis::AsyncCommands;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::config::env_opt;
use crate::modules::chat::media::MediaConfig;
use crate::modules::chat::stores;
use crate::modules::chat::sweeps::media::buckets_distinct;
use crate::modules::chat::{ChatError, ChatResult, ChatState};

pub const PARALLEL: usize = 4;
pub const PER_RUN: i64 = 120;

static MOVE: LazyLock<Option<(u16, u16)>> =
    LazyLock::new(|| env_opt("CHAT_MEDIA_MOVE").and_then(|raw| parse(&raw)));

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Moved {
    Yes,
    Skipped,
}

pub fn parse(raw: &str) -> Option<(u16, u16)> {
    let (from, to) = raw.trim().split_once(':')?;
    let pair = (from.trim().parse().ok()?, to.trim().parse().ok()?);
    (pair.0 != pair.1).then_some(pair)
}

pub fn plan(config: &MediaConfig) -> Option<(u16, u16)> {
    checked(config, (*MOVE)?)
}

pub fn checked(config: &MediaConfig, (from, to): (u16, u16)) -> Option<(u16, u16)> {
    if stores::get(config, from).is_none() || stores::get(config, to).is_none() {
        tracing::error!(
            from,
            to,
            "CHAT_MEDIA_MOVE names a store CHAT_STORES does not list"
        );
        return None;
    }
    if !buckets_distinct(config) {
        tracing::error!(
            from,
            to,
            "two chat stores share a bucket, the mover stays off"
        );
        return None;
    }
    Some((from, to))
}

pub fn cursor_key(from: u16, to: u16) -> String {
    format!("chat:media:move:{from}:{to}")
}

pub async fn run(chat: &ChatState, from: u16, to: u16) -> ChatResult<u64> {
    let key = cursor_key(from, to);
    let mut conn = chat.redis.get().await?;
    let after: Option<String> = conn.get(&key).await?;
    drop(conn);
    let picked = sqlx::query_file!(
        "queries/chat/sweeps/media_move_pick.sql",
        store_id(from)?,
        after.unwrap_or_default(),
        PER_RUN
    )
    .fetch_all(&chat.pg)
    .await?;
    let next = match picked.last() {
        Some(last) if picked.len() as i64 == PER_RUN => last.object_key.clone(),
        _ => String::new(),
    };
    let roots: Vec<(Uuid, bool)> = picked
        .iter()
        .map(|row| (row.id, row.status == "lost"))
        .collect();
    let mut outcomes = stream::iter(roots)
        .map(|(root, lost)| move_one(chat, root, lost, from, to))
        .buffer_unordered(PARALLEL);
    let mut moved = 0;
    while let Some(outcome) = outcomes.next().await {
        if outcome? == Moved::Yes {
            moved += 1;
        }
        if chat.shutdown.is_cancelled() {
            return Ok(moved);
        }
    }
    drop(outcomes);
    let mut conn = chat.redis.get().await?;
    let _: () = conn.set(&key, next).await?;
    metrics::counter!("chat_media_moved_total").increment(moved);
    Ok(moved)
}

pub async fn move_one(
    chat: &ChatState,
    root: Uuid,
    lost: bool,
    from: u16,
    to: u16,
) -> ChatResult<Moved> {
    let objects = sqlx::query_file!(
        "queries/chat/sweeps/media_move_objects.sql",
        root,
        store_id(from)?
    )
    .fetch_all(&chat.pg)
    .await?;
    if objects.is_empty() {
        return Ok(Moved::Skipped);
    }
    if !lost {
        for object in &objects {
            if !copy(chat, root, &object.object_key, from, to).await? {
                return Ok(Moved::Skipped);
            }
        }
    }
    let flipped = sqlx::query_file!(
        "queries/chat/sweeps/media_move_flip.sql",
        root,
        store_id(from)?,
        store_id(to)?
    )
    .execute(&chat.pg)
    .await?
    .rows_affected();
    let stale = if flipped > 0 {
        from
    } else {
        let now_on = sqlx::query_file_scalar!("queries/chat/sweeps/media_move_store.sql", root)
            .fetch_optional(&chat.pg)
            .await?;
        if now_on == Some(store_id(to)?) {
            return Ok(Moved::Skipped);
        }
        to
    };
    if !lost {
        for object in &objects {
            if let Err(error) = chat.s3.delete(stale, &object.object_key).await {
                tracing::warn!(%root, stale, code = error.code(), "a moved chat object left a copy for reconcile");
            }
        }
    }
    Ok(if flipped > 0 {
        Moved::Yes
    } else {
        Moved::Skipped
    })
}

async fn copy(chat: &ChatState, root: Uuid, key: &str, from: u16, to: u16) -> ChatResult<bool> {
    let Some(bytes) = chat.s3.get(from, key).await? else {
        return Ok(false);
    };
    let digest = Sha256::digest(&bytes);
    chat.s3.put(to, key, bytes).await?;
    let copied = chat.s3.get(to, key).await?;
    if copied.is_some_and(|copy| Sha256::digest(&copy) == digest) {
        return Ok(true);
    }
    tracing::error!(%root, "a moved chat object did not read back the same, it stays put");
    metrics::counter!("chat_media_move_mismatch_total").increment(1);
    Ok(false)
}

fn store_id(store: u16) -> ChatResult<i16> {
    i16::try_from(store).map_err(|_| ChatError::Internal)
}
