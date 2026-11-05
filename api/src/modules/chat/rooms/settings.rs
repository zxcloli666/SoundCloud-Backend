use std::sync::Arc;

use axum::Json;
use axum::extract::State;

use crate::error::AppError;
use crate::modules::chat::body::{ChatJson, ChatPath};
use crate::modules::chat::limits;
use crate::modules::chat::media::backdrop;
use crate::modules::chat::rooms::{perks, refusal};
use crate::modules::chat::{ChatCaller, ChatError, ChatResult, ChatState};
use crate::modules::rooms::model::{BACKDROP_COOLDOWN, Room, SettingsPatch, normalize_code};
use crate::modules::rooms::service::RoomView;

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
    let room = chat.rooms.room(code).await.map_err(refusal)?;
    let room = match fits(chat, &room, user_id, patch).await {
        Err(ChatError::RoomStarRequired) => {
            let Some(starred) = perks::upgraded(chat, &room, user_id).await? else {
                return Err(ChatError::RoomStarRequired);
            };
            fits(chat, &starred, user_id, patch).await?;
            starred
        }
        fitted => {
            fitted?;
            room
        }
    };
    let Some(wanted) = &patch.backdrop else {
        return chat
            .rooms
            .reconfigure(code, user_id, patch)
            .await
            .map_err(refusal);
    };
    if *wanted == room.backdrop {
        let unchanged = SettingsPatch {
            backdrop: None,
            ..patch.clone()
        };
        return chat
            .rooms
            .reconfigure(code, user_id, &unchanged)
            .await
            .map_err(refusal);
    }
    if let Some(media) = patch.wanted_image() {
        backdrop::check(chat, &room, user_id, media).await?;
    }
    let budget = room.id.to_string();
    limits::admit_backdrop(chat, &budget).await?;
    let changed = backdrop::change(chat, &room, user_id, patch).await;
    if changed.is_err() {
        limits::refund_backdrop(chat, &budget).await;
    }
    changed
}

async fn fits(
    chat: &ChatState,
    room: &Room,
    user_id: &str,
    patch: &SettingsPatch,
) -> ChatResult<()> {
    let cooldown = if patch.wanted_image().is_some() && room.is_host(user_id) {
        chat.rooms
            .backdrop_cooldown(user_id)
            .await
            .map_err(refusal)?
    } else {
        None
    };
    room.clone()
        .apply_settings(user_id, patch, cooldown)
        .map_err(|error| refused(error, cooldown))
}

fn refused(error: AppError, cooldown: Option<i64>) -> ChatError {
    match (&error, cooldown) {
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
