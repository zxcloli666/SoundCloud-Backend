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
use sc_chat_wire::msg::Class;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::modules::chat::body::{ChatJson, ChatPath, ChatQuery};
use crate::modules::chat::conversations::DeleteScope;
use crate::modules::chat::limits::{self, Bucket};
use crate::modules::chat::media;
use crate::modules::chat::members::access;
use crate::modules::chat::model::{MessageRow, MessageView};
use crate::modules::chat::public_chat;
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
    #[serde(default)]
    pub media: Vec<Uuid>,
    #[serde(default)]
    pub pings: Vec<String>,
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
    let class = Class::parse(&input.class).map_or("unknown", Class::as_str);
    let sent = deliver(&chat, &caller, conversation, input).await;
    let result = sent.as_ref().map_or_else(ChatError::code, |_| "ok");
    metrics::counter!("chat_send_total", "class" => class, "result" => result).increment(1);
    if conversation == public_chat::ID {
        metrics::counter!("chat_public_posts_total", "result" => result).increment(1);
    }
    sent.map(Json)
}

async fn deliver(
    chat: &Arc<ChatState>,
    caller: &ChatCaller,
    conversation: Uuid,
    input: SendMessage,
) -> ChatResult<SendAnswer> {
    if chat.config.send_paused {
        return Err(ChatError::SendPaused);
    }
    let device = caller.device.as_ref().ok_or(ChatError::Invalid)?.id;
    let outgoing = rules::outgoing(input)?;
    limits::admit_send(chat, &caller.user_id, conversation).await?;
    let access = access::load_in(chat, conversation, &caller.user_id).await?;
    rules::may_post(&access, outgoing.class)?;
    if let Err(refused) = media::refs::admit_send(chat, &access, &outgoing).await {
        let sent =
            store::existing(&chat.pg, conversation, outgoing.client_id, &caller.user_id).await?;
        return sent.map(|row| SendAnswer::from(&row)).ok_or(refused);
    }
    gate::sender_free(chat, &access).await?;
    if access.is_room()
        && let Some(row) = gate::room(chat, &access, &outgoing).await?
    {
        fanout::publish(chat, std::slice::from_ref(&row), &access.recipients).await?;
        return Ok(SendAnswer::from(&row));
    }
    if access.is_public()
        && let Some(row) = public_chat::gate::send(chat, &access, &outgoing).await?
    {
        public_chat::delivery::publish(chat, std::slice::from_ref(&row)).await?;
        return Ok(SendAnswer::from(&row));
    }
    gate::peer_open(&access)?;
    if let Some(peer) = access.peer.as_deref()
        && blocks::blocked_between(chat, &caller.user_id, peer).await?
    {
        return Err(ChatError::Unavailable);
    }
    gate::request_room(&chat.pg, &access, &outgoing).await?;
    let target = store::visible_target(&chat.pg, &access, outgoing.target_seq).await?;
    let inherited =
        rules::inherited_expiry(outgoing.class, &caller.user_id, target.as_ref(), Utc::now())?;
    let row = store::store(&chat.pg, &access, device, &outgoing, inherited).await?;
    if access.is_public() {
        public_chat::delivery::publish(chat, std::slice::from_ref(&row)).await?;
        public_chat::pings::deliver(chat, &row, &outgoing.pings, &caller.user_id).await;
    } else {
        fanout::publish(chat, std::slice::from_ref(&row), &access.recipients).await?;
    }
    Ok(SendAnswer::from(&row))
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
    if access.is_public() {
        public_chat::gate::read(&chat, &caller.user_id).await?;
    }
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
    if access.is_public() {
        public_chat::delete::after(&chat, &access, seq, &rows).await?;
        return Ok(StatusCode::NO_CONTENT);
    }
    if rows.is_empty() {
        fanout::bump(&chat, conversation, access.last_seq, &access.recipients).await?;
    } else {
        fanout::publish(&chat, &rows, &access.recipients).await?;
    }
    Ok(StatusCode::NO_CONTENT)
}
