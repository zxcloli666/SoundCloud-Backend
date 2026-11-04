use serde::Serialize;
use sqlx::PgPool;

use crate::modules::chat::members::access::Access;
use crate::modules::chat::members::roles;
use crate::modules::chat::messages::system::{self, SystemEvent};
use crate::modules::chat::model::{DISAPPEARING_TTLS, MemberState, MessageRow};
use crate::modules::chat::{ChatError, ChatResult};

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct TtlAnswer {
    pub ttl_s: Option<i32>,
    pub seq: Option<i64>,
}

pub fn checked(ttl_s: Option<i32>) -> ChatResult<Option<i32>> {
    match ttl_s {
        Some(seconds) if !DISAPPEARING_TTLS.contains(&seconds) => Err(ChatError::Invalid),
        other => Ok(other),
    }
}

pub async fn set(
    pg: &PgPool,
    access: &Access,
    ttl_s: Option<i32>,
) -> ChatResult<Option<MessageRow>> {
    if access.state != MemberState::Active {
        return Err(ChatError::NotMember);
    }
    roles::may_set_ttl(access.kind, access.role)?;
    let ttl_s = checked(ttl_s)?;
    let mut tx = pg.begin().await?;
    let changed = sqlx::query_file_scalar!(
        "queries/chat/conversations/set_ttl.sql",
        access.conversation,
        ttl_s
    )
    .fetch_optional(&mut *tx)
    .await?;
    if changed.is_none() {
        tx.rollback().await?;
        return Ok(None);
    }
    let event = SystemEvent::Ttl {
        actor: access.user_id.clone(),
        ttl_s,
    };
    let row = system::insert(&mut tx, access.conversation, &event, None).await?;
    tx.commit().await?;
    Ok(Some(row))
}

#[cfg(test)]
#[path = "../tests/conversation_ttl_tests.rs"]
mod tests;
