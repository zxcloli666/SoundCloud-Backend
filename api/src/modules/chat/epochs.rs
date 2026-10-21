use sqlx::{PgConnection, PgPool};
use uuid::Uuid;

use crate::modules::chat::ChatResult;

pub const EPOCH_MESSAGES_MAX: i32 = 10_000;
pub const EPOCH_DAYS_MAX: i64 = 30;

pub async fn mark_rekey_in(conn: &mut PgConnection, conversation: Uuid) -> ChatResult<()> {
    sqlx::query_file!("queries/chat/conversations/mark_rekey.sql", conversation)
        .execute(&mut *conn)
        .await?;
    Ok(())
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
