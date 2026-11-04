use crate::modules::chat::media::limits::{
    ALBUM_MAX, AVATAR_MAX_BYTES, IMAGE_MAX_BYTES, RESTORE_WINDOW, ROOM_ALBUM_MAX, THUMB_MAX_BYTES,
};
use crate::modules::chat::media::wire::{MediaConfigView, MediaLimitsView};
use crate::modules::chat::{ChatResult, ChatState, quota};

pub fn limits() -> MediaLimitsView {
    MediaLimitsView {
        image_max_bytes: IMAGE_MAX_BYTES as u32,
        thumb_max_bytes: THUMB_MAX_BYTES as u32,
        avatar_max_bytes: AVATAR_MAX_BYTES as u32,
        album_max: ALBUM_MAX as u32,
        room_album_max: ROOM_ALBUM_MAX as u32,
        restore_window_s: RESTORE_WINDOW.as_secs() as u32,
    }
}

pub async fn config(chat: &ChatState, user_id: &str) -> ChatResult<MediaConfigView> {
    let uploads = chat.config.media.uploads && quota::write_store_open(chat).await?;
    Ok(MediaConfigView {
        uploads,
        star: chat.stars.is_star(user_id).await?,
        quota: quota::quota(),
        used: quota::used(chat, user_id).await?.total(),
        limits: limits(),
    })
}
