use std::sync::Arc;

use axum::extract::State;
use axum::http::StatusCode;
use serde::Deserialize;
use serde_json::json;

use crate::modules::chat::body::{ChatJson, ChatPath};
use crate::modules::chat::limits::{self, Bucket};
use crate::modules::chat::members::{MemberView, roles};
use crate::modules::chat::model::{MemberRole, MemberState};
use crate::modules::chat::public_chat::{self, ID, log};
use crate::modules::chat::sync::bump;
use crate::modules::chat::users;
use crate::modules::chat::{ChatCaller, ChatError, ChatResult, ChatState};

#[derive(Debug, Clone, Deserialize)]
pub struct ModeratorBody {
    pub on: bool,
}

pub async fn standing(
    chat: &ChatState,
    user_id: &str,
) -> ChatResult<Option<(MemberRole, MemberState)>> {
    sqlx::query_file!("queries/chat/members/state_of.sql", ID, user_id)
        .fetch_optional(&chat.pg)
        .await?
        .map(|row| Ok((roles::role(&row.role)?, roles::state(&row.state)?)))
        .transpose()
}

pub async fn staff_role(chat: &ChatState, user_id: &str) -> ChatResult<MemberRole> {
    match standing(chat, user_id).await? {
        Some((role, MemberState::Active)) if roles::is_admin(role) => Ok(role),
        _ => Err(ChatError::NotMember),
    }
}

pub fn valid_target(raw: &str) -> ChatResult<()> {
    if users::valid_user_id(raw) {
        Ok(())
    } else {
        Err(ChatError::Invalid)
    }
}

pub async fn moderator(
    caller: ChatCaller,
    State(chat): State<Arc<ChatState>>,
    ChatPath(user): ChatPath<String>,
    ChatJson(body): ChatJson<ModeratorBody>,
) -> ChatResult<StatusCode> {
    public_chat::ensure_on(&chat)?;
    limits::admit(&chat, Bucket::PublicMod, &caller.user_id).await?;
    valid_target(&user)?;
    if staff_role(&chat, &caller.user_id).await? != MemberRole::Owner {
        return Err(ChatError::NotMember);
    }
    let role = if body.on {
        MemberRole::Admin
    } else {
        MemberRole::Member
    };
    let mut tx = chat.pg.begin().await?;
    let changed = sqlx::query_file_scalar!(
        "queries/chat/public/set_moderator.sql",
        ID,
        user,
        roles::role_name(role)
    )
    .fetch_optional(&mut *tx)
    .await?;
    if changed.is_none() {
        tx.rollback().await?;
        return Err(ChatError::Invalid);
    }
    let action = if body.on {
        "moderator_add"
    } else {
        "moderator_remove"
    };
    log::record_in(&mut tx, &caller.user_id, action, &user, json!({})).await?;
    tx.commit().await?;
    bump::conversation(&chat, ID, 0, std::slice::from_ref(&user)).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn listing(chat: &ChatState) -> ChatResult<Vec<MemberView>> {
    let rows = sqlx::query_file!("queries/chat/public/staff.sql", ID)
        .fetch_all(&chat.pg)
        .await?;
    Ok(rows
        .into_iter()
        .map(|row| MemberView {
            user_id: row.user_id,
            role: row.role,
            state: row.state,
            accepted: row.accepted,
            joined_seq: row.joined_seq,
            invited_by: row.invited_by,
        })
        .collect())
}
