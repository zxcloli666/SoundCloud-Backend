use sqlx::{PgConnection, PgPool};
use uuid::Uuid;

use crate::modules::chat::ChatResult;
use crate::modules::chat::model::ConversationKind;

pub const EPOCH_MESSAGES_MAX: i32 = 10_000;
pub const EPOCH_DAYS_MAX: i64 = 30;
pub const PUBLIC_EPOCH_DAYS_MAX: i64 = 7;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    pub messages: i32,
    pub days: i64,
}

pub fn limits(kind: ConversationKind) -> Limits {
    match kind {
        ConversationKind::Public => Limits {
            messages: i32::MAX,
            days: PUBLIC_EPOCH_DAYS_MAX,
        },
        _ => Limits {
            messages: EPOCH_MESSAGES_MAX,
            days: EPOCH_DAYS_MAX,
        },
    }
}

pub async fn mark_rekey_in(conn: &mut PgConnection, conversation: Uuid) -> ChatResult<()> {
    sqlx::query_file!("queries/chat/conversations/mark_rekey.sql", conversation)
        .execute(&mut *conn)
        .await?;
    Ok(())
}

pub async fn rotate_if_due(
    pg: &PgPool,
    conversation: Uuid,
    epoch: i32,
    limits: Limits,
) -> ChatResult<Option<i32>> {
    Ok(sqlx::query_file_scalar!(
        "queries/chat/conversations/rotate_epoch.sql",
        conversation,
        epoch,
        limits.messages,
        limits.days,
    )
    .fetch_optional(pg)
    .await?)
}
