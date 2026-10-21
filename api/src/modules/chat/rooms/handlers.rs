use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::modules::chat::body::{ChatJson, ChatPath, ChatQuery};
use crate::modules::chat::limits::{self, Bucket};
use crate::modules::chat::rooms::system::SuggestedTrack;
use crate::modules::chat::rooms::{bridge, refusal, suggest::suggest_track};
use crate::modules::chat::{ChatCaller, ChatError, ChatResult, ChatState};
use crate::modules::rooms::model::{SettingsPatch, normalize_code};

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

fn code_of(raw: &str) -> ChatResult<String> {
    normalize_code(raw).ok_or(ChatError::NotFound)
}

fn as_json<T: serde::Serialize>(value: &T) -> ChatResult<Json<Value>> {
    serde_json::to_value(value)
        .map(Json)
        .map_err(|_| ChatError::Internal)
}

pub async fn chat(
    caller: ChatCaller,
    State(chat): State<Arc<ChatState>>,
    ChatPath(code): ChatPath<String>,
) -> ChatResult<Json<Value>> {
    let code = code_of(&code)?;
    let opened = chat
        .rooms
        .claim_chat(&code, &caller.user_id)
        .await
        .map_err(refusal)?;
    bridge::ensure_conversation(&chat, &opened).await?;
    Ok(Json(json!({
        "conversation_id": opened.conversation_id,
        "from_seq": opened.from_seq,
    })))
}

pub async fn preview(
    caller: ChatCaller,
    State(chat): State<Arc<ChatState>>,
    ChatPath(code): ChatPath<String>,
) -> ChatResult<Json<Value>> {
    let code = code_of(&code)?;
    limits::admit(&chat, Bucket::Preview, &caller.user_id).await?;
    let preview = chat
        .rooms
        .preview(&code)
        .await
        .map_err(refusal)?
        .ok_or(ChatError::NotFound)?;
    as_json(preview.as_ref())
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
) -> ChatResult<Json<Value>> {
    let view = chat
        .rooms
        .configure(&code_of(&code)?, &caller.user_id, &patch)
        .await
        .map_err(refusal)?;
    as_json(&view)
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
