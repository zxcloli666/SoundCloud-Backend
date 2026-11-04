use uuid::Uuid;

use crate::modules::chat::model::MessageRow;
use crate::modules::chat::sync::bump as sync_bump;
use crate::modules::chat::{ChatResult, ChatState};

pub async fn publish(
    chat: &ChatState,
    rows: &[MessageRow],
    recipients: &[String],
) -> ChatResult<()> {
    let Some(last) = rows.iter().max_by_key(|row| row.seq) else {
        return Ok(());
    };
    for row in rows {
        chat.hot.put(row.clone());
    }
    bump(chat, last.conversation_id, last.seq, recipients).await
}

pub async fn bump(
    chat: &ChatState,
    conversation: Uuid,
    seq: i64,
    recipients: &[String],
) -> ChatResult<()> {
    sync_bump::conversation(chat, conversation, seq, recipients).await
}

pub async fn requests(chat: &ChatState, users: &[String]) -> ChatResult<()> {
    for user in users {
        sync_bump::requests(chat, user).await?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "../tests/fanout_tests.rs"]
mod tests;
