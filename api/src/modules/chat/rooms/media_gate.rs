use uuid::Uuid;

use crate::modules::chat::media::backdrop;
use crate::modules::chat::rooms::lookup::room_of_chat;
use crate::modules::chat::{ChatError, ChatResult, ChatState};
use crate::modules::rooms::model::{Room, RoomImages};

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

async fn room(chat: &ChatState, conversation: Uuid) -> ChatResult<Room> {
    room_of_chat(chat, conversation)
        .await?
        .ok_or(ChatError::NotMember)
}

async fn hosted(chat: &ChatState, conversation: Uuid, user_id: &str) -> ChatResult<Room> {
    let room = room(chat, conversation).await?;
    if !room.is_host(user_id) {
        return Err(ChatError::NotMember);
    }
    Ok(room)
}

pub async fn image_gate(chat: &ChatState, conversation: Uuid, user_id: &str) -> ChatResult<()> {
    let room = room(chat, conversation).await?;
    if !room.is_member(user_id) {
        return Err(ChatError::NotMember);
    }
    if !room.perks.star_host {
        return Err(ChatError::RoomStarRequired);
    }
    let allowed = match room.images_policy() {
        RoomImages::Members => chat.stars.is_star(user_id).await?,
        RoomImages::Mods => room.can_moderate(user_id),
        RoomImages::Off => false,
    };
    if !allowed {
        return Err(ChatError::Unavailable);
    }
    Ok(())
}

pub async fn backdrop_gate(chat: &ChatState, conversation: Uuid, user_id: &str) -> ChatResult<()> {
    let room = hosted(chat, conversation, user_id).await?;
    if !room.perks.star_host {
        return Err(ChatError::RoomStarRequired);
    }
    if let Some(until) = room.backdrop_cooldown(now_ms()) {
        return Err(ChatError::BackdropCooldown {
            until: u64::try_from(until).unwrap_or(0),
        });
    }
    Ok(())
}

pub async fn backdrop_meta(
    chat: &ChatState,
    conversation: Uuid,
    user_id: &str,
    media: &[Uuid],
) -> ChatResult<()> {
    let room = hosted(chat, conversation, user_id).await?;
    match media {
        [] => Ok(()),
        [id] if room.image_backdrop() == Some(*id) => Ok(()),
        [_] => Err(ChatError::MediaMissing {
            ids: media.to_vec(),
        }),
        _ => Err(ChatError::Invalid),
    }
}

pub async fn clear_backdrop_if(
    chat: &ChatState,
    conversation: Uuid,
    media: Uuid,
) -> ChatResult<bool> {
    backdrop::clear(chat, conversation, media, None).await
}
