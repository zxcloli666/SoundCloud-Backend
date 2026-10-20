use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use serde::Serialize;
use uuid::Uuid;

use crate::modules::chat::limits;
use crate::modules::chat::settings::{self, DmPolicy};
use crate::modules::chat::users::{self, Restriction, Scope};
use crate::modules::chat::{ChatCaller, ChatError, ChatResult, ChatState, blocks};

pub const MESSAGES_BEFORE_ACCEPT: i64 = 3;
pub const SESSION_MIN_AGE_SECS: i32 = 3_600;
pub const REQUESTS_SHOWN: i64 = 200;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Landing {
    Direct,
    Request,
    Unavailable,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Relation {
    pub contact: bool,
    pub follows: bool,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct RequestView {
    pub conversation_id: Uuid,
    pub kind: String,
    pub from: String,
    pub since: i64,
    pub last_seq: i64,
    pub last_at: i64,
    pub member_count: i32,
    #[serde(with = "crate::modules::chat::b64::option")]
    pub meta_ct: Option<Vec<u8>>,
    pub meta_epoch: Option<i32>,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct RequestsView {
    pub requests: Vec<RequestView>,
    pub count: usize,
}

pub async fn list(
    caller: ChatCaller,
    State(chat): State<Arc<ChatState>>,
) -> ChatResult<Json<RequestsView>> {
    let rows = sqlx::query_file!(
        "queries/chat/requests/list.sql",
        caller.user_id,
        REQUESTS_SHOWN
    )
    .fetch_all(&chat.pg)
    .await?;
    let requests: Vec<RequestView> = rows
        .into_iter()
        .map(|row| RequestView {
            conversation_id: row.id,
            kind: row.kind,
            from: row.from_user,
            since: row.updated_at.timestamp_millis(),
            last_seq: row.last_seq,
            last_at: row.last_at.timestamp_millis(),
            member_count: row.member_count,
            meta_ct: row.meta_ct,
            meta_epoch: row.meta_epoch,
        })
        .collect();
    let count = requests.len();
    Ok(Json(RequestsView { requests, count }))
}

pub async fn count(chat: &ChatState, user_id: &str) -> ChatResult<i64> {
    Ok(
        sqlx::query_file_scalar!("queries/chat/requests/count.sql", user_id)
            .fetch_one(&chat.pg)
            .await?,
    )
}

pub async fn relation(chat: &ChatState, recipient: &str, sender: &str) -> ChatResult<Relation> {
    let recipient_ids = [
        recipient.to_owned(),
        format!("soundcloud:users:{recipient}"),
    ];
    let sender_urn = format!("soundcloud:users:{sender}");
    let row = sqlx::query_file!(
        "queries/chat/requests/relation.sql",
        recipient,
        sender,
        &recipient_ids,
        sender_urn
    )
    .fetch_one(&chat.pg)
    .await?;
    Ok(Relation {
        contact: row.contact,
        follows: row.follows,
    })
}

pub async fn evaluate(chat: &ChatState, sender: &str, recipient: &str) -> ChatResult<Landing> {
    if sender == recipient {
        return Ok(Landing::Direct);
    }
    if blocks::blocked_between(chat, sender, recipient).await? {
        return Ok(Landing::Unavailable);
    }
    let policy = settings::load(chat, recipient).await?.dm_policy;
    let relation = relation(chat, recipient, sender).await?;
    Ok(landing_for(policy, relation))
}

pub fn landing_for(policy: DmPolicy, relation: Relation) -> Landing {
    let known = relation.contact || relation.follows;
    match policy {
        DmPolicy::Everyone if known => Landing::Direct,
        DmPolicy::Everyone => Landing::Request,
        DmPolicy::Following if known => Landing::Direct,
        DmPolicy::Nobody if relation.contact => Landing::Direct,
        DmPolicy::Following | DmPolicy::Nobody => Landing::Unavailable,
    }
}

pub async fn landing(
    chat: &ChatState,
    caller: &ChatCaller,
    recipient: &str,
) -> ChatResult<Landing> {
    users::check(chat, &caller.user_id, Scope::All).await?;
    match evaluate(chat, &caller.user_id, recipient).await? {
        Landing::Unavailable => Err(ChatError::Unavailable),
        Landing::Direct => Ok(Landing::Direct),
        Landing::Request => {
            users::check(chat, &caller.user_id, Scope::DmNew).await?;
            if session_is_young(chat, caller.session_id).await? {
                return Err(ChatError::TooYoung);
            }
            let newcomer = users::newcomer(chat, &caller.user_id).await?;
            limits::admit_request(chat, &caller.user_id, newcomer).await?;
            Ok(Landing::Request)
        }
    }
}

pub async fn session_is_young(chat: &ChatState, session: Uuid) -> ChatResult<bool> {
    let young = sqlx::query_file_scalar!(
        "queries/chat/requests/session_young.sql",
        session,
        SESSION_MIN_AGE_SECS
    )
    .fetch_optional(&chat.pg)
    .await?;
    Ok(young.unwrap_or(false))
}

pub async fn check_send(chat: &ChatState, conversation: Uuid, sender: &str) -> ChatResult<()> {
    let gate = sqlx::query_file!("queries/chat/requests/send_gate.sql", conversation, sender)
        .fetch_one(&chat.pg)
        .await?;
    let restriction = gate.restricted_until.map(|until| Restriction {
        until,
        scope: gate.restriction_scope.unwrap_or_default(),
    });
    if let Some(held) = &restriction
        && held.covers(Scope::All)
    {
        return Err(held.muted());
    }
    if !gate.pending {
        return Ok(());
    }
    if let Some(held) = &restriction
        && held.covers(Scope::DmNew)
    {
        return Err(held.muted());
    }
    if gate.sent >= MESSAGES_BEFORE_ACCEPT {
        return Err(ChatError::RequestLimit);
    }
    Ok(())
}

#[cfg(test)]
#[path = "tests/requests_tests.rs"]
mod tests;
