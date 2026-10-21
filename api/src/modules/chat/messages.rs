pub mod fanout;
pub mod gate;
pub mod page;
pub mod redact;
pub mod rules;
pub mod store;
pub mod system;

use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::modules::chat::body::{ChatJson, ChatPath, ChatQuery};
use crate::modules::chat::conversations::DeleteScope;
use crate::modules::chat::limits::{self, Bucket};
use crate::modules::chat::members::access;
use crate::modules::chat::model::{MessageRow, MessageView};
use crate::modules::chat::{ChatCaller, ChatError, ChatResult, ChatState, blocks};

pub const PAGE_ROWS_MAX: usize = 200;
pub const PAGE_BYTES_MAX: usize = 256 * 1024;
pub const PAGE_BYTES_MAX_PRO: usize = 24 * 1024;
pub const EDIT_WINDOW_HOURS: i64 = 48;

#[derive(Debug, Clone, Deserialize)]
pub struct SendMessage {
    pub client_id: Uuid,
    pub class: String,
    pub epoch: i32,
    pub target_seq: Option<i64>,
    #[serde(with = "crate::modules::chat::b64")]
    pub body: Vec<u8>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SendAnswer {
    pub seq: i64,
    pub ts: i64,
    pub expires_at: Option<i64>,
}

impl From<&MessageRow> for SendAnswer {
    fn from(row: &MessageRow) -> Self {
        Self {
            seq: row.seq,
            ts: row.created_at.timestamp_millis(),
            expires_at: row.expires_at.map(|at| at.timestamp_millis()),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct HistoryQuery {
    pub after: Option<i64>,
    pub before: Option<i64>,
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, Serialize)]
pub struct HistoryPage {
    pub messages: Vec<MessageView>,
    pub more: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DeleteMessageQuery {
    #[serde(rename = "for")]
    pub scope: DeleteScope,
}

pub async fn send(
    caller: ChatCaller,
    State(chat): State<Arc<ChatState>>,
    ChatPath(conversation): ChatPath<Uuid>,
    ChatJson(input): ChatJson<SendMessage>,
) -> ChatResult<Json<SendAnswer>> {
    if chat.config.send_paused {
        return Err(ChatError::SendPaused);
    }
    let device = caller.device.as_ref().ok_or(ChatError::Invalid)?.id;
    let outgoing = rules::outgoing(input)?;
    limits::admit(&chat, Bucket::Send, &caller.user_id).await?;
    let access = access::load_in(&chat, conversation, &caller.user_id).await?;
    rules::may_post(&access, outgoing.class)?;
    if access.is_room()
        && let Some(row) = gate::room(&chat, &access, &outgoing).await?
    {
        fanout::publish(&chat, std::slice::from_ref(&row), &access.recipients).await?;
        return Ok(Json(SendAnswer::from(&row)));
    }
    gate::peer_open(&access)?;
    if let Some(peer) = access.peer.as_deref()
        && blocks::blocked_between(&chat, &caller.user_id, peer).await?
    {
        return Err(ChatError::Unavailable);
    }
    gate::request_room(&chat.pg, &access, &outgoing).await?;
    let target = store::visible_target(&chat.pg, &access, outgoing.target_seq).await?;
    let inherited =
        rules::inherited_expiry(outgoing.class, &caller.user_id, target.as_ref(), Utc::now())?;
    let row = store::store(&chat.pg, &access, device, &outgoing, inherited).await?;
    fanout::publish(&chat, std::slice::from_ref(&row), &access.recipients).await?;
    Ok(Json(SendAnswer::from(&row)))
}

pub async fn history(
    caller: ChatCaller,
    State(chat): State<Arc<ChatState>>,
    ChatPath(conversation): ChatPath<Uuid>,
    ChatQuery(query): ChatQuery<HistoryQuery>,
) -> ChatResult<Json<HistoryPage>> {
    limits::admit(&chat, Bucket::ReadMessages, &caller.user_id).await?;
    let access = access::load_in(&chat, conversation, &caller.user_id)
        .await?
        .readable()?;
    let cap = page::byte_cap(caller.tier, caller.max_bytes);
    Ok(Json(page::read(&chat.pg, &access, &query, cap).await?))
}

pub async fn delete(
    caller: ChatCaller,
    State(chat): State<Arc<ChatState>>,
    ChatPath((conversation, seq)): ChatPath<(Uuid, i64)>,
    ChatQuery(query): ChatQuery<DeleteMessageQuery>,
) -> ChatResult<StatusCode> {
    if query.scope != DeleteScope::Everyone {
        return Err(ChatError::Invalid);
    }
    let access = access::load_in(&chat, conversation, &caller.user_id).await?;
    let rows = redact::for_everyone(&chat.pg, &access, seq).await?;
    if rows.is_empty() {
        fanout::bump(&chat, conversation, access.last_seq, &access.recipients).await?;
    } else {
        fanout::publish(&chat, &rows, &access.recipients).await?;
    }
    Ok(StatusCode::NO_CONTENT)
}
