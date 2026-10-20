use chat_wire::msg::Class;
use serde::Serialize;
use sqlx::PgConnection;
use uuid::Uuid;

use crate::modules::chat::model::{MemberRole, MessageRow};
use crate::modules::chat::{ChatError, ChatResult};

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SystemEvent {
    Created {
        actor: String,
    },
    Added {
        actor: String,
        targets: Vec<String>,
    },
    Joined {
        actor: String,
    },
    Left {
        actor: String,
    },
    Kicked {
        actor: String,
        target: String,
    },
    Banned {
        actor: String,
        target: String,
    },
    Role {
        actor: String,
        target: String,
        role: MemberRole,
    },
    Renamed {
        actor: String,
    },
    Ttl {
        actor: String,
        ttl_s: Option<i32>,
    },
    Deleted {
        actor: String,
    },
}

impl SystemEvent {
    pub fn actor(&self) -> &str {
        match self {
            SystemEvent::Created { actor }
            | SystemEvent::Added { actor, .. }
            | SystemEvent::Joined { actor }
            | SystemEvent::Left { actor }
            | SystemEvent::Kicked { actor, .. }
            | SystemEvent::Banned { actor, .. }
            | SystemEvent::Role { actor, .. }
            | SystemEvent::Renamed { actor }
            | SystemEvent::Ttl { actor, .. }
            | SystemEvent::Deleted { actor } => actor,
        }
    }
}

pub async fn insert(
    conn: &mut PgConnection,
    conversation: Uuid,
    event: &SystemEvent,
    target_seq: Option<i64>,
) -> ChatResult<MessageRow> {
    let body = serde_json::to_vec(event).map_err(|_| ChatError::Internal)?;
    let advanced = sqlx::query_file!("queries/chat/messages/advance_system.sql", conversation)
        .fetch_optional(&mut *conn)
        .await?
        .ok_or(ChatError::NotFound)?;
    let row = MessageRow {
        conversation_id: conversation,
        seq: advanced.last_seq,
        client_id: Uuid::now_v7(),
        sender_user: event.actor().to_owned(),
        sender_device: None,
        class: Class::System.as_str().to_owned(),
        epoch: advanced.key_epoch,
        target_seq,
        body: Some(body),
        created_at: advanced.at,
        expires_at: None,
    };
    sqlx::query_file!(
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
    .fetch_optional(&mut *conn)
    .await?
    .ok_or(ChatError::Internal)?;
    Ok(row)
}

#[cfg(test)]
#[path = "../tests/system_event_tests.rs"]
mod tests;
