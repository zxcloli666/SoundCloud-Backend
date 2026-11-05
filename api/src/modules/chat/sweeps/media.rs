pub mod delete;
pub mod expire;
pub mod gauges;
pub mod holds;
pub mod mover;
pub mod reconcile;
pub mod recount;
pub mod trim;
pub mod withdrawn;

use std::collections::HashSet;

use sqlx::PgConnection;
use uuid::Uuid;

use crate::modules::chat::media::{MediaConfig, ledger};
use crate::modules::chat::sweeps::budget::Budget;
use crate::modules::chat::{ChatError, ChatResult, ChatState};

pub const EVERY_SECS: u64 = 30;
pub const BATCH_REFS: i64 = 5_000;

pub async fn run_once(chat: &ChatState) -> ChatResult<u64> {
    let mut run = Run::default();
    run.step(
        "expire",
        expire::run(chat, &mut Budget::new(expire::BATCHES)).await,
    );
    run.step(
        "trim",
        trim::run(chat, &mut Budget::new(trim::BATCHES)).await,
    );
    run.step(
        "withdrawn",
        withdrawn::run(chat, &mut Budget::new(withdrawn::BATCHES)).await,
    );
    run.step(
        "holds",
        holds::run(chat, &mut Budget::new(holds::BATCHES)).await,
    );
    run.step(
        "delete",
        delete::run(chat, &mut Budget::new(delete::CLAIMS)).await,
    );
    let now = (chat.config.clock)();
    run.step(
        "reconcile",
        reconcile::run(chat, now, &mut Budget::new(reconcile::PAGES_PER_RUN)).await,
    );
    if let Some((from, to)) = chat.config.media.moving {
        let mut budget = Budget::new(mover::PER_RUN as u32);
        run.step("move", mover::run(chat, from, to, &mut budget).await);
    }
    run.step("gauges", gauges::refresh(chat).await.map(|()| 0));
    run.finish()
}

#[derive(Default)]
struct Run {
    done: u64,
    failed: Option<ChatError>,
}

impl Run {
    fn step(&mut self, step: &'static str, outcome: ChatResult<u64>) {
        match outcome {
            Ok(items) => self.done += items,
            Err(error) => {
                tracing::warn!(step, code = error.code(), "a chat media step failed");
                self.failed.get_or_insert(error);
            }
        }
    }

    fn finish(self) -> ChatResult<u64> {
        match self.failed {
            Some(error) => Err(error),
            None => Ok(self.done),
        }
    }
}

pub async fn purge_refs(
    chat: &ChatState,
    conversation: Uuid,
    budget: &mut Budget,
) -> ChatResult<bool> {
    if !refs_left(chat, conversation).await? {
        return Ok(true);
    }
    loop {
        if !budget.spend(chat) {
            return Ok(false);
        }
        let mut tx = chat.pg.begin().await?;
        let roots = sqlx::query_file_scalar!(
            "queries/chat/sweeps/media_purge_refs.sql",
            conversation,
            BATCH_REFS
        )
        .fetch_all(&mut *tx)
        .await?;
        let full = roots.len() as i64 == BATCH_REFS;
        ledger::settle(&mut tx, &roots).await?;
        tx.commit().await?;
        if !full && !refs_left(chat, conversation).await? {
            return Ok(true);
        }
    }
}

async fn refs_left(chat: &ChatState, conversation: Uuid) -> ChatResult<bool> {
    Ok(
        sqlx::query_file_scalar!("queries/chat/sweeps/media_refs_left.sql", conversation)
            .fetch_one(&chat.pg)
            .await?,
    )
}

pub fn buckets_distinct(config: &MediaConfig) -> bool {
    let buckets: HashSet<&str> = config
        .stores
        .iter()
        .map(|store| store.bucket.as_str())
        .collect();
    buckets.len() == config.stores.len()
}

pub async fn room_ended(tx: &mut PgConnection, conversation: Uuid) -> ChatResult<()> {
    let roots = sqlx::query_file_scalar!(
        "queries/chat/sweeps/media_room_ended_lock.sql",
        conversation
    )
    .fetch_all(&mut *tx)
    .await?;
    if roots.is_empty() {
        return Ok(());
    }
    ledger::lock_owners(tx, &roots).await?;
    sqlx::query_file!("queries/chat/sweeps/media_room_ended.sql", &roots)
        .execute(&mut *tx)
        .await?;
    Ok(())
}

#[cfg(test)]
#[path = "../tests/media_sweep/fixtures.rs"]
pub mod fixtures;

#[cfg(test)]
#[path = "../tests/media_sweep/s3_live/mod.rs"]
mod s3_live;
