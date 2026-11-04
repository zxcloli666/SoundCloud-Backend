use std::sync::Arc;

use axum::Json;
use axum::extract::State;

use crate::error::AppError;
use crate::modules::chat::body::{ChatJson, ChatPath};
use crate::modules::chat::limits;
use crate::modules::chat::media::backdrop;
use crate::modules::chat::rooms::refusal;
use crate::modules::chat::{ChatCaller, ChatError, ChatResult, ChatState};
use crate::modules::rooms::model::{BACKDROP_COOLDOWN, Room, SettingsPatch, normalize_code};
use crate::modules::rooms::service::RoomView;

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

pub async fn settings(
    caller: ChatCaller,
    State(chat): State<Arc<ChatState>>,
    ChatPath(code): ChatPath<String>,
    ChatJson(patch): ChatJson<SettingsPatch>,
) -> ChatResult<Json<RoomView>> {
    let code = normalize_code(&code).ok_or(ChatError::NotFound)?;
    let after = configure(&chat, &code, &caller.user_id, &patch).await?;
    let view = chat
        .rooms
        .show(after, &caller.user_id)
        .await
        .map_err(refusal)?;
    Ok(Json(view))
}

pub async fn configure(
    chat: &ChatState,
    code: &str,
    user_id: &str,
    patch: &SettingsPatch,
) -> ChatResult<Room> {
    let Some(wanted) = &patch.backdrop else {
        return chat
            .rooms
            .reconfigure(code, user_id, patch)
            .await
            .map_err(refusal);
    };
    let room = chat.rooms.room(code).await.map_err(refusal)?;
    room.clone()
        .apply_settings(user_id, patch, now_ms())
        .map_err(|error| refused(&room, error))?;
    if *wanted == room.backdrop {
        let unchanged = SettingsPatch {
            backdrop: None,
            ..patch.clone()
        };
        return chat
            .rooms
            .reconfigure(code, user_id, &unchanged)
            .await
            .map_err(|error| refused(&room, error));
    }
    if let Some(media) = patch.wanted_image() {
        backdrop::check(chat, &room, user_id, media).await?;
    }
    limits::admit_backdrop(chat, &limit_key(&room)).await?;
    match backdrop::change(chat, &room, user_id, patch).await? {
        Ok(after) => Ok(after),
        Err(error) => match chat.rooms.find(code).await {
            Ok(Some(now)) => Err(refused(&now, error)),
            _ => Err(refusal(error)),
        },
    }
}

fn limit_key(room: &Room) -> String {
    room.chat_id
        .map_or_else(|| room.code.clone(), |chat| chat.to_string())
}

fn refused(room: &Room, error: AppError) -> ChatError {
    match (&error, room.backdrop_cooldown(now_ms())) {
        (
            AppError::Coded {
                code: BACKDROP_COOLDOWN,
                ..
            },
            Some(until),
        ) => ChatError::BackdropCooldown {
            until: u64::try_from(until).unwrap_or(0),
        },
        _ => refusal(error),
    }
}
