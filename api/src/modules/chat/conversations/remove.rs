use sqlx::{PgConnection, PgPool};
use uuid::Uuid;

use crate::modules::chat::members::access::Access;
use crate::modules::chat::members::roles;
use crate::modules::chat::model::{ConversationKind, MemberState};
use crate::modules::chat::{ChatError, ChatResult};

pub async fn hide(pg: &PgPool, access: &Access) -> ChatResult<i64> {
    sqlx::query_file_scalar!(
        "queries/chat/members/hide.sql",
        access.conversation,
        access.user_id
    )
    .fetch_optional(pg)
    .await?
    .ok_or(ChatError::NotMember)
}

pub async fn for_everyone(pg: &PgPool, access: &Access) -> ChatResult<bool> {
    if access.state != MemberState::Active {
        return Err(ChatError::NotMember);
    }
    roles::may_delete_for_everyone(access.kind, access.role)?;
    if access.kind == ConversationKind::Saved {
        hide(pg, access).await?;
        return Ok(false);
    }
    let mut tx = pg.begin().await?;
    let deleted = shred(&mut tx, access.conversation).await?;
    tx.commit().await?;
    Ok(deleted)
}

pub async fn shred(conn: &mut PgConnection, conversation: Uuid) -> ChatResult<bool> {
    let deleted = sqlx::query_file_scalar!("queries/chat/conversations/delete.sql", conversation)
        .fetch_optional(&mut *conn)
        .await?;
    if deleted.is_none() {
        return Ok(false);
    }
    sqlx::query_file!("queries/chat/conversations/shred_keys.sql", conversation)
        .execute(&mut *conn)
        .await?;
    Ok(true)
}
