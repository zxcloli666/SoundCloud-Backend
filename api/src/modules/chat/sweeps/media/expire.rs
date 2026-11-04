use crate::modules::chat::sweeps::budget::Budget;
use crate::modules::chat::{ChatResult, ChatState};

pub const BATCH: i64 = 500;
pub const BATCHES: u32 = 20;

pub async fn run(chat: &ChatState, budget: &mut Budget) -> ChatResult<u64> {
    let mut expired = 0;
    while budget.spend(chat) {
        let moved = sqlx::query_file_scalar!("queries/chat/sweeps/media_expire.sql", BATCH)
            .fetch_one(&chat.pg)
            .await?;
        expired += u64::try_from(moved).unwrap_or(0);
        if moved < BATCH {
            break;
        }
    }
    Ok(expired)
}
