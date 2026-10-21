pub mod create;
#[cfg(test)]
#[path = "tests/lane_test_fixtures.rs"]
pub mod fixtures;
pub mod keys;
pub mod patch;
pub mod remove;
pub mod ttl;
pub mod view;

#[cfg(test)]
#[path = "tests/conversation_routes_tests.rs"]
mod route_tests;

use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use serde::Deserialize;
use uuid::Uuid;

use crate::modules::chat::body::{ChatJson, ChatPath, ChatQuery};
use crate::modules::chat::conversations::create::{Created, DirectPlan, Wanted};
use crate::modules::chat::conversations::patch::{ConversationPatch, MyPatch};
use crate::modules::chat::conversations::ttl::TtlAnswer;
use crate::modules::chat::conversations::view::{ConversationPage, ConversationView};
use crate::modules::chat::limits::{self, Bucket};
use crate::modules::chat::members::{access, changes, contacts};
use crate::modules::chat::messages::fanout;
use crate::modules::chat::model::ConversationKind;
use crate::modules::chat::users::{self, Scope};
use crate::modules::chat::{ChatCaller, ChatError, ChatResult, ChatState, blocks, requests};

pub const LIST_ROWS_MAX: usize = 200;
pub const LIST_BYTES_MAX: usize = 128 * 1024;

#[derive(Debug, Clone, Deserialize)]
pub struct CreateConversation {
    pub kind: ConversationKind,
    #[serde(default)]
    pub members: Vec<String>,
    #[serde(default, with = "crate::modules::chat::b64::option")]
    pub meta_ct: Option<Vec<u8>>,
    pub meta_epoch: Option<i32>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SetTtl {
    pub ttl_s: Option<i32>,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DeleteScope {
    Me,
    Everyone,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DeleteQuery {
    #[serde(rename = "for")]
    pub scope: DeleteScope,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ListQuery {
    pub cursor: Option<Uuid>,
}

pub async fn list(
    caller: ChatCaller,
    State(chat): State<Arc<ChatState>>,
    ChatQuery(query): ChatQuery<ListQuery>,
) -> ChatResult<Json<ConversationPage>> {
    limits::admit(&chat, Bucket::ReadList, &caller.user_id).await?;
    let fetch = i64::try_from(LIST_ROWS_MAX + 1).map_err(|_| ChatError::Internal)?;
    let rows = view::rows_after(
        &chat.pg,
        &caller.user_id,
        query.cursor.unwrap_or(Uuid::nil()),
        fetch,
    )
    .await?;
    let cap = match usize::try_from(caller.max_bytes) {
        Ok(asked) if asked > 0 => asked.min(LIST_BYTES_MAX),
        _ => LIST_BYTES_MAX,
    };
    Ok(Json(view::fit(rows, &caller.user_id, LIST_ROWS_MAX, cap)))
}

pub async fn create(
    caller: ChatCaller,
    State(chat): State<Arc<ChatState>>,
    ChatJson(input): ChatJson<CreateConversation>,
) -> ChatResult<Json<ConversationView>> {
    let user = caller.user_id.as_str();
    let wanted = create::wanted(user, input, chat.config.loadtest)?;
    limits::admit(&chat, Bucket::Create, user).await?;
    if wanted != Wanted::Saved {
        users::check(&chat, user, Scope::All).await?;
    }
    let created = match wanted {
        Wanted::Saved => create::saved(&chat.pg, user).await?,
        Wanted::Direct(peer) => {
            if blocks::blocked_between(&chat, user, &peer).await? {
                return Err(ChatError::Unavailable);
            }
            create_direct(&chat, &caller, &peer).await?
        }
        Wanted::Group { members, meta } => {
            create::under_cap(&chat.pg, user).await?;
            let open = contacts::unblocked(&chat, user, members).await?;
            let placed = contacts::placements(&chat.pg, user, &open).await?;
            contacts::admit_invites(&chat, user, &placed).await?;
            create::group(&chat.pg, user, placed, meta).await?
        }
    };
    let view = view::one(&chat.pg, created.id, user).await?;
    if created.rows.is_empty() {
        fanout::bump(&chat, created.id, view.last_seq, &created.recipients).await?;
    } else {
        fanout::publish(&chat, &created.rows, &created.recipients).await?;
    }
    Ok(Json(view))
}

async fn create_direct(chat: &ChatState, caller: &ChatCaller, peer: &str) -> ChatResult<Created> {
    let user = caller.user_id.as_str();
    let plan = create::plan_direct(&chat.pg, user, peer).await?;
    if matches!(plan, DirectPlan::New { .. }) {
        create::under_cap(&chat.pg, user).await?;
    }
    if plan.is_request() {
        admit_request(chat, caller).await?;
    }
    create::direct(&chat.pg, user, plan).await
}

pub async fn admit_request(chat: &ChatState, caller: &ChatCaller) -> ChatResult<()> {
    users::check(chat, &caller.user_id, Scope::DmNew).await?;
    if requests::session_is_young(chat, caller.session_id).await? {
        return Err(ChatError::TooYoung);
    }
    let newcomer = users::newcomer(chat, &caller.user_id).await?;
    limits::admit_request(chat, &caller.user_id, newcomer).await
}

pub async fn show(
    caller: ChatCaller,
    State(chat): State<Arc<ChatState>>,
    ChatPath(conversation): ChatPath<Uuid>,
) -> ChatResult<Json<ConversationView>> {
    Ok(Json(
        view::one(&chat.pg, conversation, &caller.user_id).await?,
    ))
}

pub async fn update(
    caller: ChatCaller,
    State(chat): State<Arc<ChatState>>,
    ChatPath(conversation): ChatPath<Uuid>,
    ChatJson(input): ChatJson<ConversationPatch>,
) -> ChatResult<Json<ConversationView>> {
    let access = access::load(&chat.pg, conversation, &caller.user_id).await?;
    let renamed = patch::apply(&chat.pg, &access, input).await?;
    match renamed {
        Some(row) => fanout::publish(&chat, &[row], &access.recipients).await?,
        None => fanout::bump(&chat, conversation, access.last_seq, &access.recipients).await?,
    }
    Ok(Json(
        view::one(&chat.pg, conversation, &caller.user_id).await?,
    ))
}

pub async fn delete(
    caller: ChatCaller,
    State(chat): State<Arc<ChatState>>,
    ChatPath(conversation): ChatPath<Uuid>,
    ChatQuery(query): ChatQuery<DeleteQuery>,
) -> ChatResult<StatusCode> {
    let access = access::load(&chat.pg, conversation, &caller.user_id).await?;
    match (query.scope, access.kind) {
        (_, ConversationKind::Room) => return Err(ChatError::Invalid),
        (DeleteScope::Me, ConversationKind::Group) => {
            let change = changes::leave(&chat.pg, &access).await?;
            if change.deleted {
                chat.hot.forget(conversation);
            }
            changes::publish(&chat, &access, change).await?;
        }
        (DeleteScope::Me, _) => {
            let hidden = remove::hide(&chat.pg, &access).await?;
            let me = std::slice::from_ref(&caller.user_id);
            fanout::bump(&chat, conversation, hidden, me).await?;
        }
        (DeleteScope::Everyone, _) => {
            if remove::for_everyone(&chat.pg, &access).await? {
                chat.hot.forget(conversation);
            }
            fanout::bump(&chat, conversation, access.last_seq, &access.recipients).await?;
        }
    }
    Ok(StatusCode::NO_CONTENT)
}

pub async fn update_me(
    caller: ChatCaller,
    State(chat): State<Arc<ChatState>>,
    ChatPath(conversation): ChatPath<Uuid>,
    ChatJson(input): ChatJson<MyPatch>,
) -> ChatResult<Json<ConversationView>> {
    let access = access::load(&chat.pg, conversation, &caller.user_id)
        .await?
        .readable()?;
    patch::apply_mine(&chat.pg, &access, input).await?;
    fanout::bump(
        &chat,
        conversation,
        access.last_seq,
        std::slice::from_ref(&caller.user_id),
    )
    .await?;
    Ok(Json(
        view::one(&chat.pg, conversation, &caller.user_id).await?,
    ))
}

pub async fn set_ttl(
    caller: ChatCaller,
    State(chat): State<Arc<ChatState>>,
    ChatPath(conversation): ChatPath<Uuid>,
    ChatJson(input): ChatJson<SetTtl>,
) -> ChatResult<Json<TtlAnswer>> {
    let ttl_s = ttl::checked(input.ttl_s)?;
    let access = access::load(&chat.pg, conversation, &caller.user_id).await?;
    let row = ttl::set(&chat.pg, &access, ttl_s).await?;
    match &row {
        Some(row) => fanout::publish(&chat, std::slice::from_ref(row), &access.recipients).await?,
        None => fanout::bump(&chat, conversation, access.last_seq, &access.recipients).await?,
    }
    Ok(Json(TtlAnswer {
        ttl_s,
        seq: row.map(|row| row.seq),
    }))
}
