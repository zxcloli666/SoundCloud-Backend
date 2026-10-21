use uuid::Uuid;

use crate::modules::chat::{ChatError, ChatResult, ChatState};

pub async fn ensure_current(chat: &ChatState, conversation: Uuid) -> ChatResult<i32> {
    advance(chat, conversation, false).await
}

#[cfg(test)]
pub async fn rotate(chat: &ChatState, conversation: Uuid) -> ChatResult<i32> {
    advance(chat, conversation, true).await
}

async fn advance(chat: &ChatState, conversation: Uuid, force: bool) -> ChatResult<i32> {
    let mut tx = chat.pg.begin().await?;
    let state = sqlx::query_file!("queries/chat/keys/lock_conversation.sql", conversation)
        .fetch_optional(&mut *tx)
        .await?
        .filter(|state| !state.deleted)
        .ok_or(ChatError::NotFound)?;
    let moves = force || state.rekey;
    let epoch = if moves {
        state.key_epoch.checked_add(1).ok_or(ChatError::Internal)?
    } else {
        state.key_epoch
    };
    let ck: [u8; 32] = chat_wire::random_bytes().map_err(|_| ChatError::Internal)?;
    let wrapped = chat.keks.wrap(conversation, epoch, &ck)?;
    let kek_version = i16::try_from(chat.keks.current()).map_err(|_| ChatError::Internal)?;
    sqlx::query_file!(
        "queries/chat/keys/insert.sql",
        conversation,
        epoch,
        wrapped,
        kek_version
    )
    .execute(&mut *tx)
    .await?;
    if moves {
        sqlx::query_file!("queries/chat/keys/advance.sql", conversation, epoch)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok(epoch)
}
