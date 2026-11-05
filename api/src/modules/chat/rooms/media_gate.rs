use uuid::Uuid;

use crate::modules::chat::rooms::lookup::room_of_chat;
use crate::modules::chat::rooms::{perks, refusal};
use crate::modules::chat::{ChatError, ChatResult, ChatState};
use crate::modules::rooms::model::{Room, RoomImages};

async fn room(chat: &ChatState, conversation: Uuid) -> ChatResult<Room> {
    room_of_chat(chat, conversation)
        .await?
        .ok_or(ChatError::NotMember)
}

async fn starred(chat: &ChatState, room: Room, user_id: &str) -> ChatResult<Room> {
    if room.perks.star_host {
        return Ok(room);
    }
    perks::upgraded(chat, &room, user_id)
        .await?
        .ok_or(ChatError::RoomStarRequired)
}

pub async fn image_gate(chat: &ChatState, conversation: Uuid, user_id: &str) -> ChatResult<()> {
    let room = room(chat, conversation).await?;
    if !room.is_member(user_id) {
        return Err(ChatError::NotMember);
    }
    let room = starred(chat, room, user_id).await?;
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
    let room = room(chat, conversation).await?;
    if !room.is_host(user_id) {
        return Err(ChatError::NotMember);
    }
    starred(chat, room, user_id).await?;
    let cooldown = chat
        .rooms
        .backdrop_cooldown(user_id)
        .await
        .map_err(refusal)?;
    if let Some(until) = cooldown {
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
    let room = room(chat, conversation).await?;
    if !room.is_host(user_id) {
        return Err(ChatError::NotMember);
    }
    match media {
        [] => Ok(()),
        [id] if room.image_backdrop() == Some(*id) => Ok(()),
        [_] => Err(ChatError::MediaMissing {
            ids: media.to_vec(),
        }),
        _ => Err(ChatError::Invalid),
    }
}
