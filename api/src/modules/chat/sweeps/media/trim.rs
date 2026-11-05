use uuid::Uuid;

use crate::modules::chat::media::ledger;
use crate::modules::chat::sweeps::budget::Budget;
use crate::modules::chat::sweeps::media::BATCH_REFS;
use crate::modules::chat::{ChatResult, ChatState};

pub const CONVERSATIONS: i64 = 32;
pub const BATCHES: u32 = 40;

pub async fn run(chat: &ChatState, budget: &mut Budget) -> ChatResult<u64> {
    let queued = sqlx::query_file!("queries/chat/sweeps/media_trim_queue.sql", CONVERSATIONS)
        .fetch_all(&chat.pg)
        .await?;
    let mut dropped = 0;
    for entry in queued {
        let Some(refs) = conversation(chat, entry.conversation_id, budget).await? else {
            break;
        };
        dropped += refs;
        sqlx::query_file!(
            "queries/chat/sweeps/media_trim_done.sql",
            entry.conversation_id,
            entry.queued_at
        )
        .execute(&chat.pg)
        .await?;
    }
    Ok(dropped)
}

async fn conversation(
    chat: &ChatState,
    conversation: Uuid,
    budget: &mut Budget,
) -> ChatResult<Option<u64>> {
    let floor = sqlx::query_file_scalar!("queries/chat/sweeps/media_trim_floor.sql", conversation)
        .fetch_one(&chat.pg)
        .await?;
    let Some(floor) = floor else {
        return Ok(Some(0));
    };
    let mut dropped = 0;
    loop {
        if !budget.spend(chat) {
            return Ok(None);
        }
        let mut tx = chat.pg.begin().await?;
        let roots = sqlx::query_file_scalar!(
            "queries/chat/sweeps/media_trim_refs.sql",
            conversation,
            floor,
            BATCH_REFS
        )
        .fetch_all(&mut *tx)
        .await?;
        let full = roots.len() as i64 == BATCH_REFS;
        dropped += roots.len() as u64;
        ledger::settle(&mut tx, &roots).await?;
        tx.commit().await?;
        if !full && !refs_left(chat, conversation, floor).await? {
            return Ok(Some(dropped));
        }
    }
}

async fn refs_left(chat: &ChatState, conversation: Uuid, floor: i64) -> ChatResult<bool> {
    Ok(sqlx::query_file_scalar!(
        "queries/chat/sweeps/media_trim_left.sql",
        conversation,
        floor
    )
    .fetch_one(&chat.pg)
    .await?)
}
