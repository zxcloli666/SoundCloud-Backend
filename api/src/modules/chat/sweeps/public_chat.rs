use std::time::Duration;

use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::modules::chat::media::ledger;
use crate::modules::chat::public_chat::{ID, RETENTION_DAYS, watchers};
use crate::modules::chat::sweeps::budget::Budget;
use crate::modules::chat::sweeps::gauges;
use crate::modules::chat::{ChatResult, ChatState};

pub const EVERY_SECS: u64 = 30;
pub const BATCH_ROWS: i64 = 500;
pub const BATCHES_PER_RUN: u32 = 20;
pub const BATCH_PAUSE: Duration = Duration::from_millis(100);

pub async fn run_once(chat: &ChatState) -> ChatResult<u64> {
    watchers::current(chat).await?;
    sqlx::query_file!("queries/chat/public/purge_mutes.sql")
        .execute(&chat.pg)
        .await?;
    let cutoff = Utc::now() - chrono::Duration::days(RETENTION_DAYS);
    let mut budget = Budget::new(BATCHES_PER_RUN);
    let mut deleted = 0;
    while budget.spend(chat) {
        let gone = delete_batch(chat, cutoff).await?;
        deleted += gone;
        if gone < BATCH_ROWS as u64 {
            break;
        }
        tokio::time::sleep(BATCH_PAUSE).await;
    }
    let oldest = sqlx::query_file_scalar!("queries/chat/public/oldest.sql", ID)
        .fetch_optional(&chat.pg)
        .await?
        .filter(|at| *at < cutoff);
    gauges::backlog(
        "public",
        oldest.map(|at| at + chrono::Duration::days(RETENTION_DAYS)),
    );
    Ok(deleted)
}

pub async fn delete_batch(chat: &ChatState, cutoff: DateTime<Utc>) -> ChatResult<u64> {
    let mut tx = chat.pg.begin().await?;
    let rows = sqlx::query_file!("queries/chat/public/purge_old.sql", ID, cutoff, BATCH_ROWS)
        .fetch_all(&mut *tx)
        .await?;
    let roots: Vec<Uuid> = rows
        .iter()
        .flat_map(|row| row.media.iter().flatten().copied())
        .collect();
    ledger::settle(&mut tx, &roots).await?;
    tx.commit().await?;
    Ok(rows.len() as u64)
}
