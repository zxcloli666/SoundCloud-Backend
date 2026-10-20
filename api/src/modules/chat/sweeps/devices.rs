use std::collections::BTreeMap;

use uuid::Uuid;

use crate::modules::chat::sweeps::budget::Budget;
use crate::modules::chat::sweeps::hooks;
use crate::modules::chat::{ChatResult, ChatState};

pub const EVERY_SECS: u64 = 3_600;
pub const BATCH_ROWS: i64 = 500;
pub const BATCHES_PER_RUN: u32 = 20;

pub async fn run_once(chat: &ChatState) -> ChatResult<u64> {
    let mut budget = Budget::new(BATCHES_PER_RUN);
    let mut revoked = 0;
    while budget.spend(chat) {
        let batch = revoke_batch(chat).await?;
        let count = batch.values().map(Vec::len).sum::<usize>();
        revoked += count as u64;
        for (user_id, ids) in &batch {
            hooks::devices_revoked(chat, user_id, ids).await?;
        }
        if (count as i64) < BATCH_ROWS {
            break;
        }
    }
    Ok(revoked)
}

async fn revoke_batch(chat: &ChatState) -> ChatResult<BTreeMap<String, Vec<Uuid>>> {
    let mut tx = chat.pg.begin().await?;
    let rows = sqlx::query_file!("queries/chat/sweeps/revoke_sessionless.sql", BATCH_ROWS)
        .fetch_all(&mut *tx)
        .await?;
    let mut revoked: BTreeMap<String, Vec<Uuid>> = BTreeMap::new();
    for row in rows {
        revoked.entry(row.user_id).or_default().push(row.id);
    }
    if !revoked.is_empty() {
        let users: Vec<String> = revoked.keys().cloned().collect();
        sqlx::query_file!("queries/chat/sweeps/rekey_users.sql", &users)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok(revoked)
}
