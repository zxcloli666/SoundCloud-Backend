use std::sync::Arc;

use axum::Json;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::StatusCode;
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chrono::{Duration, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::modules::chat::body::ChatPath;
use crate::modules::chat::limits::{self, Bucket};
use crate::modules::chat::model::GROUP_MEMBERS_MAX;
use crate::modules::chat::users::{self, Scope};
use crate::modules::chat::{ChatCaller, ChatError, ChatResult, ChatState, blocks};

pub const DEFAULT_TTL_SECS: i64 = 7 * 86_400;
pub const MIN_TTL_SECS: i64 = 300;
pub const MAX_TTL_SECS: i64 = 30 * 86_400;
pub const USES_MAX: i32 = 1_000;

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateInvite {
    pub expires_in_s: Option<i64>,
    pub uses: Option<i32>,
}

#[derive(Clone, Serialize, PartialEq)]
pub struct InviteView {
    pub token: String,
    pub conversation_id: Uuid,
    pub expires_at: i64,
    pub uses_left: Option<i32>,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct JoinAnswer {
    pub conversation_id: Uuid,
    pub joined: bool,
}

pub async fn create(
    caller: ChatCaller,
    State(chat): State<Arc<ChatState>>,
    ChatPath(conversation): ChatPath<Uuid>,
    body: Bytes,
) -> ChatResult<Json<InviteView>> {
    let input: CreateInvite = if body.is_empty() {
        CreateInvite::default()
    } else {
        serde_json::from_slice(&body).map_err(|_| ChatError::Invalid)?
    };
    limits::admit(&chat, Bucket::Invite, &caller.user_id).await?;
    Ok(Json(
        create_with(&chat, &caller.user_id, conversation, &input).await?,
    ))
}

pub async fn create_with(
    chat: &ChatState,
    user_id: &str,
    conversation: Uuid,
    input: &CreateInvite,
) -> ChatResult<InviteView> {
    let ttl = input.expires_in_s.unwrap_or(DEFAULT_TTL_SECS);
    if !(MIN_TTL_SECS..=MAX_TTL_SECS).contains(&ttl) {
        return Err(ChatError::Invalid);
    }
    if input
        .uses
        .is_some_and(|uses| !(1..=USES_MAX).contains(&uses))
    {
        return Err(ChatError::Invalid);
    }
    require_group_admin(chat, conversation, user_id).await?;
    users::check(chat, user_id, Scope::All).await?;

    let secret: [u8; 32] = chat_wire::random_bytes().map_err(|_| ChatError::Internal)?;
    let expires_at = Utc::now() + Duration::seconds(ttl);
    let hash = token_hash(&secret);
    sqlx::query_file!(
        "queries/chat/invites/insert.sql",
        hash.as_slice(),
        conversation,
        user_id,
        expires_at,
        input.uses.unwrap_or(0)
    )
    .execute(&chat.pg)
    .await?;
    Ok(InviteView {
        token: URL_SAFE_NO_PAD.encode(secret),
        conversation_id: conversation,
        expires_at: expires_at.timestamp_millis(),
        uses_left: input.uses,
    })
}

pub async fn revoke(
    caller: ChatCaller,
    State(chat): State<Arc<ChatState>>,
    ChatPath(token): ChatPath<String>,
) -> ChatResult<StatusCode> {
    let hash = parse_token(&token).ok_or(ChatError::NotFound)?;
    sqlx::query_file_scalar!(
        "queries/chat/invites/revoke.sql",
        hash.as_slice(),
        caller.user_id
    )
    .fetch_optional(&chat.pg)
    .await?
    .ok_or(ChatError::NotFound)?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn join(
    caller: ChatCaller,
    State(chat): State<Arc<ChatState>>,
    ChatPath(token): ChatPath<String>,
) -> ChatResult<Json<JoinAnswer>> {
    limits::admit(&chat, Bucket::Create, &caller.user_id).await?;
    let answer = join_with(&chat, &caller.user_id, &token).await?;
    if answer.joined {
        chat.hub.wake(&caller.user_id);
    }
    Ok(Json(answer))
}

pub async fn join_with(chat: &ChatState, user_id: &str, token: &str) -> ChatResult<JoinAnswer> {
    let hash = parse_token(token).ok_or(ChatError::NotFound)?;
    let invite = sqlx::query_file!("queries/chat/invites/find.sql", hash.as_slice())
        .fetch_optional(&chat.pg)
        .await?
        .ok_or(ChatError::NotFound)?;
    let state = sqlx::query_file_scalar!(
        "queries/chat/invites/membership.sql",
        invite.conversation_id,
        user_id
    )
    .fetch_optional(&chat.pg)
    .await?;
    match state.as_deref() {
        Some("active") => {
            return Ok(JoinAnswer {
                conversation_id: invite.conversation_id,
                joined: false,
            });
        }
        Some("banned") => return Err(ChatError::Unavailable),
        _ => {}
    }
    if invite.uses_left.is_some_and(|uses| uses <= 0) {
        return Err(ChatError::NotFound);
    }
    users::check(chat, user_id, Scope::All).await?;
    if blocks::blocked_between(chat, user_id, &invite.created_by).await? {
        return Err(ChatError::Unavailable);
    }
    Ok(JoinAnswer {
        conversation_id: admit_member(chat, &hash, user_id).await?,
        joined: true,
    })
}

async fn admit_member(chat: &ChatState, hash: &[u8; 32], user_id: &str) -> ChatResult<Uuid> {
    let mut tx = chat.pg.begin().await?;
    let used = sqlx::query_file!("queries/chat/invites/consume.sql", hash.as_slice())
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(ChatError::NotFound)?;
    let group = sqlx::query_file!("queries/chat/invites/lock_group.sql", used.conversation_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(ChatError::NotFound)?;
    if group.member_count as usize >= GROUP_MEMBERS_MAX {
        return Err(ChatError::Unavailable);
    }
    sqlx::query_file_scalar!(
        "queries/chat/invites/add_member.sql",
        used.conversation_id,
        user_id,
        group.last_seq,
        group.key_epoch,
        used.created_by
    )
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(ChatError::Unavailable)?;
    sqlx::query_file!("queries/chat/invites/recount.sql", used.conversation_id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(used.conversation_id)
}

async fn require_group_admin(
    chat: &ChatState,
    conversation: Uuid,
    user_id: &str,
) -> ChatResult<()> {
    let member = sqlx::query_file!(
        "queries/chat/invites/member_role.sql",
        conversation,
        user_id
    )
    .fetch_optional(&chat.pg)
    .await?
    .ok_or(ChatError::NotMember)?;
    if !member.live || member.state != "active" {
        return Err(ChatError::NotMember);
    }
    if member.kind != "group" || !matches!(member.role.as_str(), "owner" | "admin") {
        return Err(ChatError::Invalid);
    }
    Ok(())
}

pub fn token_hash(secret: &[u8]) -> [u8; 32] {
    Sha256::digest(secret).into()
}

pub fn parse_token(token: &str) -> Option<[u8; 32]> {
    let secret = URL_SAFE_NO_PAD.decode(token).ok()?;
    (secret.len() == 32).then(|| token_hash(&secret))
}

#[cfg(test)]
#[path = "tests/invites_tests.rs"]
mod tests;
