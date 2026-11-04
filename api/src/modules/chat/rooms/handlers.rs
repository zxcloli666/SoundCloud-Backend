use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::modules::chat::body::{ChatJson, ChatPath, ChatQuery};
use crate::modules::chat::limits::{self, Bucket};
use crate::modules::chat::rooms::system::SuggestedTrack;
use crate::modules::chat::rooms::{bridge, refusal, suggest::suggest_track};
use crate::modules::chat::{ChatCaller, ChatError, ChatResult, ChatState};
use crate::modules::rooms::model::{SettingsPatch, normalize_code};
use crate::modules::rooms::service::{RoomPreview, RoomView};

#[derive(Debug, Clone, Default, Deserialize)]
pub struct KickQuery {
    #[serde(default)]
    pub ban: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MuteBody {
    pub seconds: u32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ModeratorBody {
    pub on: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SuggestBody {
    pub track: Value,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct RoomChatView {
    pub conversation_id: Uuid,
    pub from_seq: i64,
}

fn code_of(raw: &str) -> ChatResult<String> {
    normalize_code(raw).ok_or(ChatError::NotFound)
}

pub async fn chat(
    caller: ChatCaller,
    State(chat): State<Arc<ChatState>>,
    ChatPath(code): ChatPath<String>,
) -> ChatResult<Json<RoomChatView>> {
    let code = code_of(&code)?;
    let opened = chat
        .rooms
        .claim_chat(&code, &caller.user_id)
        .await
        .map_err(refusal)?;
    bridge::ensure_conversation(&chat, &opened).await?;
    Ok(Json(RoomChatView {
        conversation_id: opened.conversation_id,
        from_seq: opened.from_seq,
    }))
}

pub async fn preview(
    caller: ChatCaller,
    State(chat): State<Arc<ChatState>>,
    ChatPath(code): ChatPath<String>,
) -> ChatResult<Json<RoomPreview>> {
    let code = code_of(&code)?;
    limits::admit(&chat, Bucket::Preview, &caller.user_id).await?;
    let preview = chat
        .rooms
        .preview(&code)
        .await
        .map_err(refusal)?
        .ok_or(ChatError::NotFound)?;
    Ok(Json(preview.as_ref().clone()))
}

pub async fn remove_member(
    caller: ChatCaller,
    State(chat): State<Arc<ChatState>>,
    ChatPath((code, user)): ChatPath<(String, String)>,
    ChatQuery(query): ChatQuery<KickQuery>,
) -> ChatResult<StatusCode> {
    chat.rooms
        .kick(&code_of(&code)?, &caller.user_id, &user, query.ban)
        .await
        .map_err(refusal)?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn mute(
    caller: ChatCaller,
    State(chat): State<Arc<ChatState>>,
    ChatPath((code, user)): ChatPath<(String, String)>,
    ChatJson(body): ChatJson<MuteBody>,
) -> ChatResult<StatusCode> {
    chat.rooms
        .mute(&code_of(&code)?, &caller.user_id, &user, body.seconds)
        .await
        .map_err(refusal)?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn moderator(
    caller: ChatCaller,
    State(chat): State<Arc<ChatState>>,
    ChatPath((code, user)): ChatPath<(String, String)>,
    ChatJson(body): ChatJson<ModeratorBody>,
) -> ChatResult<StatusCode> {
    chat.rooms
        .set_moderator(&code_of(&code)?, &caller.user_id, &user, body.on)
        .await
        .map_err(refusal)?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn settings(
    caller: ChatCaller,
    State(chat): State<Arc<ChatState>>,
    ChatPath(code): ChatPath<String>,
    ChatJson(patch): ChatJson<SettingsPatch>,
) -> ChatResult<Json<RoomView>> {
    let view = chat
        .rooms
        .configure(&code_of(&code)?, &caller.user_id, &patch)
        .await
        .map_err(refusal)?;
    Ok(Json(view))
}

pub async fn suggest(
    caller: ChatCaller,
    State(chat): State<Arc<ChatState>>,
    ChatPath(code): ChatPath<String>,
    ChatJson(body): ChatJson<SuggestBody>,
) -> ChatResult<StatusCode> {
    let code = code_of(&code)?;
    let track = SuggestedTrack::from_snapshot(&body.track).ok_or(ChatError::Invalid)?;
    suggest_track(&chat, &code, &caller.user_id, track).await?;
    Ok(StatusCode::NO_CONTENT)
}
