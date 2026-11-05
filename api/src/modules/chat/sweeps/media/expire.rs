use crate::modules::chat::media::ledger;
use crate::modules::chat::sweeps::budget::Budget;
use crate::modules::chat::{ChatResult, ChatState};

pub const BATCH: i64 = 500;
pub const BATCHES: u32 = 20;

pub async fn run(chat: &ChatState, budget: &mut Budget) -> ChatResult<u64> {
    let mut expired = 0;
    while budget.spend(chat) {
        let mut tx = chat.pg.begin().await?;
        let due = sqlx::query_file_scalar!("queries/chat/sweeps/media_expire_due.sql", BATCH)
            .fetch_all(&mut *tx)
            .await?;
        if due.is_empty() {
            tx.rollback().await?;
            break;
        }
        ledger::lock_owners(&mut tx, &due).await?;
        let moved = sqlx::query_file_scalar!("queries/chat/sweeps/media_expire.sql", &due)
            .fetch_one(&mut *tx)
            .await?;
        tx.commit().await?;
        expired += u64::try_from(moved).unwrap_or(0);
        if (due.len() as i64) < BATCH {
            break;
        }
    }
    Ok(expired)
}
