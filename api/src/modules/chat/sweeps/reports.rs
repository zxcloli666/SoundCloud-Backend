use crate::modules::chat::sweeps::budget::Budget;
use crate::modules::chat::{ChatResult, ChatState};

pub const EVERY_SECS: u64 = 3_600;
pub const BATCH_ROWS: i64 = 1_000;
pub const BATCHES_PER_RUN: u32 = 10;

pub async fn run_once(chat: &ChatState) -> ChatResult<u64> {
    let mut budget = Budget::new(BATCHES_PER_RUN);
    let mut purged = 0;
    while budget.spend(chat) {
        let done = sqlx::query_file!("queries/chat/sweeps/drop_evidence.sql", BATCH_ROWS)
            .execute(&chat.pg)
            .await?
            .rows_affected();
        purged += done;
        if done < BATCH_ROWS as u64 {
            break;
        }
    }
    Ok(purged)
}
