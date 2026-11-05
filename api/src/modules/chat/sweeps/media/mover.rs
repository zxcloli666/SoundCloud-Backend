use futures::{StreamExt, future, stream};
use redis::AsyncCommands;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::modules::chat::media::ledger;
use crate::modules::chat::sweeps::budget::Budget;
use crate::modules::chat::{ChatError, ChatResult, ChatState};

pub const PARALLEL: usize = 4;
pub const PER_RUN: i64 = 120;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Moved {
    Yes,
    Skipped,
}

pub fn cursor_key(from: u16, to: u16) -> String {
    format!("chat:media:move:{from}:{to}")
}

pub async fn run(chat: &ChatState, from: u16, to: u16, budget: &mut Budget) -> ChatResult<u64> {
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
    let roots: Vec<(Uuid, bool)> = picked
        .iter()
        .map(|row| (row.id, row.status == "lost"))
        .collect();
    let mut launched = 0;
    let mut moved = 0;
    let mut failed = 0;
    let mut outcomes = stream::iter(roots)
        .take_while(|_| future::ready(budget.spend(chat)))
        .inspect(|_| launched += 1)
        .map(|(root, lost)| async move { (root, move_one(chat, root, lost, from, to).await) })
        .buffer_unordered(PARALLEL);
    while let Some((root, outcome)) = outcomes.next().await {
        match outcome {
            Ok(Moved::Yes) => moved += 1,
            Ok(Moved::Skipped) => {}
            Err(error) => {
                failed += 1;
                tracing::warn!(%root, from, to, code = error.code(), "a chat object did not move, the next pass retries it");
            }
        }
    }
    drop(outcomes);
    let next = if launched < picked.len() {
        launched
            .checked_sub(1)
            .map(|last| picked[last].object_key.clone())
    } else if picked.len() as i64 == PER_RUN {
        picked.last().map(|last| last.object_key.clone())
    } else {
        Some(String::new())
    };
    if let Some(next) = next {
        let mut conn = chat.redis.get().await?;
        let _: () = conn.set(&key, next).await?;
    }
    metrics::counter!("chat_media_moved_total").increment(moved);
    metrics::counter!("chat_media_move_failed_total").increment(failed);
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
    let flipped = flip(chat, root, from, to).await?;
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

async fn flip(chat: &ChatState, root: Uuid, from: u16, to: u16) -> ChatResult<u64> {
    let mut tx = chat.pg.begin().await?;
    ledger::lock(&mut tx, &[root]).await?;
    let flipped = sqlx::query_file!(
        "queries/chat/sweeps/media_move_flip.sql",
        root,
        store_id(from)?,
        store_id(to)?
    )
    .execute(&mut *tx)
    .await?
    .rows_affected();
    tx.commit().await?;
    Ok(flipped)
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
