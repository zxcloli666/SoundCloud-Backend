use crate::modules::chat::sweeps::budget::Budget;
use crate::modules::chat::sweeps::media::settle;
use crate::modules::chat::{ChatResult, ChatState};

pub const BATCH: i64 = 5_000;
pub const BATCHES: u32 = 10;

pub async fn run(chat: &ChatState, budget: &mut Budget) -> ChatResult<u64> {
    let mut settled = 0;
    while budget.spend(chat) {
        let mut tx = chat.pg.begin().await?;
        let roots = sqlx::query_file_scalar!("queries/chat/sweeps/media_holds_drop.sql", BATCH)
            .fetch_all(&mut *tx)
            .await?;
        let full = roots.len() as i64 == BATCH;
        settled += settle(&mut tx, roots).await?;
        tx.commit().await?;
        if !full {
            break;
        }
    }
    while budget.spend(chat) {
        let mut tx = chat.pg.begin().await?;
        let roots = sqlx::query_file_scalar!("queries/chat/sweeps/media_held_free.sql", BATCH)
            .fetch_all(&mut *tx)
            .await?;
        let full = roots.len() as i64 == BATCH;
        let freed = settle(&mut tx, roots).await?;
        tx.commit().await?;
        settled += freed;
        if !full || freed == 0 {
            break;
        }
    }
    Ok(settled)
}
