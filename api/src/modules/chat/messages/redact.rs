use chrono::Utc;
use sc_chat_wire::msg::Class;
use sqlx::PgPool;

use crate::modules::chat::members::access::Access;
use crate::modules::chat::messages::store;
use crate::modules::chat::messages::system::{self, SystemEvent};
use crate::modules::chat::model::{MemberState, MessageRow};
use crate::modules::chat::{ChatError, ChatResult};

pub async fn for_everyone(pg: &PgPool, access: &Access, seq: i64) -> ChatResult<Vec<MessageRow>> {
    if access.state != MemberState::Active {
        return Err(ChatError::NotMember);
    }
    let target = store::visible_target(pg, access, Some(seq))
        .await?
        .ok_or(ChatError::NotFound)?;
    if target.class != Class::Post.as_str() {
        return Err(ChatError::Invalid);
    }
    if target.expires_at.is_some_and(|at| at <= Utc::now()) {
        return Err(ChatError::NotFound);
    }
    if target.sender_user != access.user_id && !access.moderates() {
        return Err(ChatError::NotMember);
    }
    if target.deleted {
        return Ok(Vec::new());
    }
    let mut tx = pg.begin().await?;
    let mut rows = sqlx::query_file_as!(
        MessageRow,
        "queries/chat/messages/redact.sql",
        access.conversation,
        seq
    )
    .fetch_all(&mut *tx)
    .await?;
    if rows.is_empty() {
        tx.rollback().await?;
        return Ok(rows);
    }
    let event = SystemEvent::Deleted {
        actor: access.user_id.clone(),
    };
    let deleted = system::insert_until(
        &mut tx,
        access.conversation,
        &event,
        Some(seq),
        target.expires_at,
    )
    .await?;
    rows.push(deleted);
    tx.commit().await?;
    Ok(rows)
}

#[cfg(test)]
#[path = "../tests/message_redact_tests.rs"]
mod tests;
