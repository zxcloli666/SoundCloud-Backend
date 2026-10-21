use std::time::Duration;

use uuid::Uuid;

use crate::modules::chat::sweeps::budget::Budget;
use crate::modules::chat::sweeps::gauges;
use crate::modules::chat::{ChatResult, ChatState, rt_keys};

pub const BATCH_ROWS: i64 = 5_000;
pub const EVERY_SECS: u64 = 30;
pub const BATCHES_PER_RUN: u32 = 40;
pub const CONVERSATIONS_PER_RUN: i64 = 32;
pub const BATCH_PAUSE: Duration = Duration::from_millis(20);

pub async fn run_once(chat: &ChatState) -> ChatResult<u64> {
    let mut budget = Budget::new(BATCHES_PER_RUN);
    let pending = sqlx::query_file_scalar!(
        "queries/chat/sweeps/purge_pending.sql",
        CONVERSATIONS_PER_RUN
    )
    .fetch_all(&chat.pg)
    .await?;
    let mut purged = 0;
    for conversation in pending {
        if !budget.left(chat) {
            break;
        }
        if self::conversation(chat, conversation, &mut budget).await? {
            purged += 1;
        }
    }
    let oldest = sqlx::query_file_scalar!("queries/chat/sweeps/purge_backlog.sql")
        .fetch_one(&chat.pg)
        .await?;
    gauges::backlog("purge", oldest);
    Ok(purged)
}

pub async fn shred(chat: &ChatState, conversation: Uuid) -> ChatResult<bool> {
    let mut tx = chat.pg.begin().await?;
    let shredded = sqlx::query_file!("queries/chat/sweeps/shred_room.sql", conversation)
        .execute(&mut *tx)
        .await?
        .rows_affected()
        == 1;
    if shredded {
        sqlx::query_file!("queries/chat/sweeps/purge_keys.sql", conversation)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    chat.hot.forget(conversation);
    Ok(shredded)
}

pub async fn conversation(
    chat: &ChatState,
    conversation: Uuid,
    budget: &mut Budget,
) -> ChatResult<bool> {
    sqlx::query_file!("queries/chat/sweeps/purge_keys.sql", conversation)
        .execute(&chat.pg)
        .await?;
    loop {
        if !budget.spend(chat) {
            return Ok(false);
        }
        let deleted = sqlx::query_file!(
            "queries/chat/sweeps/purge_messages.sql",
            conversation,
            BATCH_ROWS
        )
        .execute(&chat.pg)
        .await?
        .rows_affected();
        if deleted < BATCH_ROWS as u64 {
            break;
        }
        tokio::time::sleep(BATCH_PAUSE).await;
    }
    sqlx::query_file!("queries/chat/sweeps/purge_invites.sql", conversation)
        .execute(&chat.pg)
        .await?;
    sqlx::query_file!("queries/chat/sweeps/purge_members.sql", conversation)
        .execute(&chat.pg)
        .await?;
    let gone = sqlx::query_file!("queries/chat/sweeps/purge_conversation.sql", conversation)
        .execute(&chat.pg)
        .await?
        .rows_affected()
        == 1;
    if gone {
        chat.hot.forget(conversation);
        forget_marks(chat, conversation).await?;
    }
    Ok(gone)
}

async fn forget_marks(chat: &ChatState, conversation: Uuid) -> ChatResult<()> {
    let mut conn = chat.redis.get().await?;
    let _: i64 = redis::cmd("DEL")
        .arg(rt_keys::conversation_seq(conversation))
        .arg(rt_keys::read_marks(conversation))
        .arg(rt_keys::delivered_marks(conversation))
        .query_async(&mut conn)
        .await?;
    Ok(())
}
