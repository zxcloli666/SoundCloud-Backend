pub mod access;
pub mod changes;
pub mod contacts;
pub mod roles;
pub mod store;

use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::modules::chat::body::{ChatJson, ChatPath, ChatQuery};
use crate::modules::chat::conversations::create::group_members;
use crate::modules::chat::model::{MemberRole, MemberState};
use crate::modules::chat::users::{self, Scope};
use crate::modules::chat::{ChatCaller, ChatError, ChatResult, ChatState};

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct MemberView {
    pub user_id: String,
    pub role: String,
    pub state: String,
    pub accepted: bool,
    pub joined_seq: i64,
    pub invited_by: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AddMembers {
    pub users: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RemoveQuery {
    #[serde(default)]
    pub ban: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UpdateMember {
    pub role: Option<MemberRole>,
    #[serde(default)]
    pub unban: bool,
}

pub async fn list(
    caller: ChatCaller,
    State(chat): State<Arc<ChatState>>,
    ChatPath(conversation): ChatPath<Uuid>,
) -> ChatResult<Json<Vec<MemberView>>> {
    let access = access::load(&chat.pg, conversation, &caller.user_id)
        .await?
        .readable()?;
    let admin = access.is_admin();
    let rows = sqlx::query_file!("queries/chat/members/list.sql", conversation)
        .fetch_all(&chat.pg)
        .await?;
    Ok(Json(
        rows.into_iter()
            .filter(|row| admin || row.state != roles::state_name(MemberState::Banned))
            .map(|row| MemberView {
                user_id: row.user_id,
                role: row.role,
                state: row.state,
                accepted: row.accepted,
                joined_seq: row.joined_seq,
                invited_by: row.invited_by,
            })
            .collect(),
    ))
}

pub async fn add(
    caller: ChatCaller,
    State(chat): State<Arc<ChatState>>,
    ChatPath(conversation): ChatPath<Uuid>,
    ChatJson(input): ChatJson<AddMembers>,
) -> ChatResult<Json<Vec<String>>> {
    let user = caller.user_id.as_str();
    let access = access::load(&chat.pg, conversation, user)
        .await?
        .active()?
        .admin()?;
    let wanted = group_members(user, &input.users, chat.config.loadtest)?;
    users::check(&chat, user, Scope::All).await?;
    let open = contacts::unblocked(&chat, user, wanted).await?;
    let placed = contacts::placements(&chat.pg, user, &open).await?;
    contacts::admit_invites(&chat, user, &placed).await?;
    let change = changes::add(&chat.pg, &access, placed).await?;
    let added = change.added.clone();
    changes::publish(&chat, &access, change).await?;
    Ok(Json(added))
}

pub async fn remove(
    caller: ChatCaller,
    State(chat): State<Arc<ChatState>>,
    ChatPath((conversation, user)): ChatPath<(Uuid, String)>,
    ChatQuery(query): ChatQuery<RemoveQuery>,
) -> ChatResult<StatusCode> {
    let access = access::load(&chat.pg, conversation, &caller.user_id).await?;
    let change = if user == caller.user_id {
        if query.ban {
            return Err(ChatError::Invalid);
        }
        changes::leave(&chat.pg, &access).await?
    } else {
        changes::remove(&chat.pg, &access, &user, query.ban).await?
    };
    if change.deleted {
        chat.hot.forget(conversation);
    }
    changes::publish(&chat, &access, change).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn update(
    caller: ChatCaller,
    State(chat): State<Arc<ChatState>>,
    ChatPath((conversation, user)): ChatPath<(Uuid, String)>,
    ChatJson(input): ChatJson<UpdateMember>,
) -> ChatResult<Json<Vec<MemberView>>> {
    let access = access::load(&chat.pg, conversation, &caller.user_id).await?;
    let change = match (input.role, input.unban) {
        (Some(role), false) => changes::set_role(&chat.pg, &access, &user, role).await?,
        (None, true) => changes::unban(&chat.pg, &access, &user).await?,
        _ => return Err(ChatError::Invalid),
    };
    changes::publish(&chat, &access, change).await?;
    list(caller, State(chat), ChatPath(conversation)).await
}

pub async fn accept(
    caller: ChatCaller,
    State(chat): State<Arc<ChatState>>,
    ChatPath(conversation): ChatPath<Uuid>,
) -> ChatResult<StatusCode> {
    let access = access::load(&chat.pg, conversation, &caller.user_id).await?;
    let change = changes::accept(&chat.pg, &access).await?;
    changes::publish(&chat, &access, change).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn decline(
    caller: ChatCaller,
    State(chat): State<Arc<ChatState>>,
    ChatPath(conversation): ChatPath<Uuid>,
) -> ChatResult<StatusCode> {
    let access = access::load(&chat.pg, conversation, &caller.user_id).await?;
    let change = changes::decline(&chat.pg, &access).await?;
    changes::publish(&chat, &access, change).await?;
    Ok(StatusCode::NO_CONTENT)
}
