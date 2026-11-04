use crate::modules::chat::sweeps::budget::Budget;
use crate::modules::chat::sweeps::media::settle;
use crate::modules::chat::{ChatResult, ChatState};

pub const BATCH: i64 = 500;
pub const BATCHES: u32 = 10;

pub async fn run(chat: &ChatState, budget: &mut Budget) -> ChatResult<u64> {
    let mut withdrawn = 0;
    while budget.spend(chat) {
        let mut tx = chat.pg.begin().await?;
        let due = sqlx::query_file_scalar!("queries/chat/sweeps/media_withdrawn_due.sql", BATCH)
            .fetch_all(&mut *tx)
            .await?;
        if due.is_empty() {
            tx.rollback().await?;
            break;
        }
        sqlx::query_file_scalar!("queries/chat/sweeps/media_withdrawn_refs.sql", &due)
            .fetch_all(&mut *tx)
            .await?;
        settle(&mut tx, due.clone()).await?;
        sqlx::query_file_scalar!("queries/chat/sweeps/media_withdrawn_hold.sql", &due)
            .fetch_all(&mut *tx)
            .await?;
        tx.commit().await?;
        withdrawn += due.len() as u64;
        if (due.len() as i64) < BATCH {
            break;
        }
    }
    Ok(withdrawn)
}
