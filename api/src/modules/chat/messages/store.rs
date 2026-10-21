use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use crate::modules::chat::epochs::{self, EPOCH_DAYS_MAX, EPOCH_MESSAGES_MAX};
use crate::modules::chat::members::access::Access;
use crate::modules::chat::messages::rules::{self, Outgoing, Target};
use crate::modules::chat::model::MessageRow;
use crate::modules::chat::{ChatError, ChatResult};

pub async fn target(pg: &PgPool, conversation: Uuid, seq: i64) -> ChatResult<Option<Target>> {
    Ok(
        sqlx::query_file!("queries/chat/messages/target.sql", conversation, seq)
            .fetch_optional(pg)
            .await?
            .map(|row| Target {
                sender_user: row.sender_user,
                class: row.class,
                created_at: row.created_at,
                expires_at: row.expires_at,
                deleted: row.deleted,
            }),
    )
}

pub async fn visible_target(
    pg: &PgPool,
    access: &Access,
    seq: Option<i64>,
) -> ChatResult<Option<Target>> {
    match seq {
        Some(seq) if seq > access.history_floor() => target(pg, access.conversation, seq).await,
        _ => Ok(None),
    }
}

pub async fn existing(
    pg: &PgPool,
    conversation: Uuid,
    client_id: Uuid,
    user: &str,
) -> ChatResult<Option<MessageRow>> {
    let row = sqlx::query_file_as!(
        MessageRow,
        "queries/chat/messages/by_client.sql",
        conversation,
        client_id
    )
    .fetch_optional(pg)
    .await?;
    match row {
        Some(row) if row.sender_user != user => Err(ChatError::Invalid),
        other => Ok(other),
    }
}

pub async fn store(
    pg: &PgPool,
    access: &Access,
    device: Uuid,
    out: &Outgoing,
    inherited: Option<DateTime<Utc>>,
) -> ChatResult<MessageRow> {
    let conversation = access.conversation;
    let mut tx = pg.begin().await?;
    let advanced = sqlx::query_file!(
        "queries/chat/messages/advance.sql",
        conversation,
        out.epoch,
        out.class.as_str(),
        out.body,
        EPOCH_MESSAGES_MAX,
        EPOCH_DAYS_MAX,
    )
    .fetch_optional(&mut *tx)
    .await?;
    let Some(advanced) = advanced else {
        tx.rollback().await?;
        return stale(pg, access, out).await;
    };
    let row = MessageRow {
        conversation_id: conversation,
        seq: advanced.last_seq,
        client_id: out.client_id,
        sender_user: access.user_id.clone(),
        sender_device: Some(device),
        class: out.class.as_str().to_owned(),
        epoch: out.epoch,
        target_seq: out.target_seq,
        body: Some(out.body.clone()),
        created_at: advanced.at,
        expires_at: rules::expiry(out.class, advanced.ttl_s, inherited, advanced.at),
    };
    let inserted = sqlx::query_file!(
        "queries/chat/messages/insert.sql",
        row.conversation_id,
        row.seq,
        row.client_id,
        row.sender_user,
        row.sender_device,
        row.class,
        row.epoch,
        row.target_seq,
        row.body,
        row.created_at,
        row.expires_at,
    )
    .fetch_optional(&mut *tx)
    .await?;
    if inserted.is_none() {
        tx.rollback().await?;
        return existing(pg, conversation, out.client_id, &access.user_id)
            .await?
            .ok_or(ChatError::Internal);
    }
    if !access.accepted && out.class == chat_wire::msg::Class::Post {
        sqlx::query_file!(
            "queries/chat/members/mark_accepted.sql",
            conversation,
            access.user_id
        )
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(row)
}

async fn stale(pg: &PgPool, access: &Access, out: &Outgoing) -> ChatResult<MessageRow> {
    if let Some(row) = existing(pg, access.conversation, out.client_id, &access.user_id).await? {
        return Ok(row);
    }
    epochs::rotate_if_due(pg, access.conversation, out.epoch).await?;
    Err(ChatError::EpochStale)
}

#[cfg(test)]
#[path = "../tests/message_store_tests.rs"]
mod tests;
