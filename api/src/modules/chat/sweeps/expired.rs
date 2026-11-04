use std::collections::{BTreeMap, HashMap};
use std::time::Duration;

use uuid::Uuid;

use crate::modules::chat::sweeps::budget::Budget;
use crate::modules::chat::sweeps::gauges;
use crate::modules::chat::{ChatResult, ChatState, sync};

pub const BATCH_ROWS: i64 = 5_000;
pub const EVERY_SECS: u64 = 60;
pub const BATCHES_PER_RUN: u32 = 20;
pub const BATCH_PAUSE: Duration = Duration::from_millis(20);
pub const CONVERSATIONS_PER_QUERY: usize = 200;

pub async fn run_once(chat: &ChatState) -> ChatResult<u64> {
    let mut budget = Budget::new(BATCHES_PER_RUN);
    let mut deleted = 0;
    while budget.spend(chat) {
        let gone = delete_batch(chat).await?;
        deleted += gone.len() as u64;
        let full = gone.len() as i64 == BATCH_ROWS;
        announce(chat, by_conversation(gone)).await?;
        if !full {
            break;
        }
        tokio::time::sleep(BATCH_PAUSE).await;
    }
    let oldest = sqlx::query_file_scalar!("queries/chat/sweeps/expired_backlog.sql")
        .fetch_one(&chat.pg)
        .await?;
    gauges::backlog("expired", oldest);
    Ok(deleted)
}

pub fn by_conversation(rows: Vec<(Uuid, i64)>) -> BTreeMap<Uuid, Vec<i64>> {
    let mut grouped: BTreeMap<Uuid, Vec<i64>> = BTreeMap::new();
    for (conversation, seq) in rows {
        grouped.entry(conversation).or_default().push(seq);
    }
    for seqs in grouped.values_mut() {
        seqs.sort_unstable();
        seqs.dedup();
    }
    grouped
}

async fn delete_batch(chat: &ChatState) -> ChatResult<Vec<(Uuid, i64)>> {
    let rows = sqlx::query_file!("queries/chat/sweeps/expired_delete.sql", BATCH_ROWS)
        .fetch_all(&chat.pg)
        .await?;
    Ok(rows
        .into_iter()
        .map(|row| (row.conversation_id, row.seq))
        .collect())
}

async fn announce(chat: &ChatState, gone: BTreeMap<Uuid, Vec<i64>>) -> ChatResult<()> {
    let ids: Vec<Uuid> = gone.keys().copied().collect();
    for chunk in ids.chunks(CONVERSATIONS_PER_QUERY) {
        let mut recipients: HashMap<Uuid, Vec<String>> = HashMap::new();
        for row in sqlx::query_file!("queries/chat/sweeps/live_members.sql", chunk)
            .fetch_all(&chat.pg)
            .await?
        {
            recipients
                .entry(row.conversation_id)
                .or_default()
                .push(row.user_id);
        }
        for conversation in chunk {
            let users = recipients.remove(conversation).unwrap_or_default();
            sync::bump::messages_gone(chat, *conversation, &gone[conversation], &users).await?;
        }
    }
    Ok(())
}
