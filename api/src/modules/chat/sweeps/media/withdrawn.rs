use uuid::Uuid;

use crate::modules::chat::media::backdrop;
use crate::modules::chat::sweeps::budget::Budget;
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
        let backdrops: Vec<(Uuid, Uuid)> =
            sqlx::query_file!("queries/chat/sweeps/media_withdrawn_refs.sql", &due)
                .fetch_all(&mut *tx)
                .await?
                .into_iter()
                .filter(|row| row.backdrop)
                .map(|row| (row.conversation_id, row.media_id))
                .collect();
        sqlx::query_file!("queries/chat/sweeps/media_withdrawn_hold.sql", &due)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        for (conversation, media) in backdrops {
            take_down(chat, conversation, media).await;
        }
        withdrawn += due.len() as u64;
        if (due.len() as i64) < BATCH {
            break;
        }
    }
    Ok(withdrawn)
}

async fn take_down(chat: &ChatState, conversation: Uuid, media: Uuid) {
    match backdrop::clear(chat, conversation, media, None).await {
        Ok(true) => {
            metrics::counter!("chat_backdrops_cleared_total", "by" => "owner_delete").increment(1);
        }
        Ok(false) => {}
        Err(error) => {
            metrics::counter!("chat_backdrop_clear_failures_total").increment(1);
            tracing::warn!(
                %conversation,
                code = error.code(),
                "a deleted room backdrop stays up until the next change"
            );
        }
    }
}
