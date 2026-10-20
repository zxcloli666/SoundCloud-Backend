use sqlx::{PgConnection, PgPool};
use uuid::Uuid;

use crate::modules::chat::{ChatError, ChatResult, ChatState};

pub const EPOCH_MESSAGES_MAX: i32 = 10_000;
pub const EPOCH_DAYS_MAX: i64 = 30;

pub async fn current_epoch(chat: &ChatState, conversation: Uuid) -> ChatResult<i32> {
    sqlx::query_file_scalar!("queries/chat/conversations/key_epoch.sql", conversation)
        .fetch_optional(&chat.pg)
        .await?
        .ok_or(ChatError::NotFound)
}

pub async fn mark_rekey(chat: &ChatState, conversation: Uuid) -> ChatResult<()> {
    let mut conn = chat.pg.acquire().await?;
    mark_rekey_in(&mut conn, conversation).await
}

pub async fn mark_rekey_in(conn: &mut PgConnection, conversation: Uuid) -> ChatResult<()> {
    sqlx::query_file!("queries/chat/conversations/mark_rekey.sql", conversation)
        .execute(&mut *conn)
        .await?;
    Ok(())
}

pub async fn mark_rekey_for_user(chat: &ChatState, user_id: &str) -> ChatResult<u64> {
    Ok(
        sqlx::query_file!("queries/chat/conversations/mark_rekey_user.sql", user_id)
            .execute(&chat.pg)
            .await?
            .rows_affected(),
    )
}

pub async fn rotate_if_due(pg: &PgPool, conversation: Uuid, epoch: i32) -> ChatResult<Option<i32>> {
    Ok(sqlx::query_file_scalar!(
        "queries/chat/conversations/rotate_epoch.sql",
        conversation,
        epoch,
        EPOCH_MESSAGES_MAX,
        EPOCH_DAYS_MAX,
    )
    .fetch_optional(pg)
    .await?)
}
