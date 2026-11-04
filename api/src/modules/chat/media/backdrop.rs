pub mod lock;
mod tidy;

use uuid::Uuid;

use crate::error::AppError;
use crate::modules::chat::rooms::lookup::room_of_chat;
use crate::modules::chat::rooms::refusal;
use crate::modules::chat::{ChatError, ChatResult, ChatState};
use crate::modules::rooms::model::{Room, SettingsPatch};

use lock::Held;
pub use tidy::Attached;
#[cfg(test)]
pub use tidy::tidy;

const KIND: &str = "backdrop";

struct Upload {
    owner_user: String,
    kind: String,
    status: String,
    origin_conversation: Uuid,
}

pub async fn check(chat: &ChatState, room: &Room, user_id: &str, media: Uuid) -> ChatResult<()> {
    let upload = sqlx::query_file_as!(Upload, "queries/chat/backdrop/peek_media.sql", media)
        .fetch_optional(&chat.pg)
        .await?;
    usable(upload, room.chat_id, user_id, media)
}

fn usable(
    upload: Option<Upload>,
    conversation: Option<Uuid>,
    user_id: &str,
    media: Uuid,
) -> ChatResult<()> {
    let fits = upload.is_some_and(|upload| {
        upload.kind == KIND
            && upload.owner_user == user_id
            && Some(upload.origin_conversation) == conversation
            && matches!(upload.status.as_str(), "uploaded" | "attached")
    });
    if fits {
        Ok(())
    } else {
        Err(ChatError::MediaMissing { ids: vec![media] })
    }
}

pub async fn change(
    chat: &ChatState,
    room: &Room,
    user_id: &str,
    patch: &SettingsPatch,
) -> ChatResult<Result<Room, AppError>> {
    let Some(conversation) = room.chat_id else {
        if let Some(media) = patch.wanted_image() {
            return Err(ChatError::MediaMissing { ids: vec![media] });
        }
        return Ok(chat.rooms.reconfigure(&room.code, user_id, patch).await);
    };
    let held = Held::take(chat, conversation).await?;
    let attached = match patch.wanted_image() {
        Some(media) => match attach(chat, &held, conversation, user_id, media).await {
            Ok(attached) => Some(attached),
            Err(error) => {
                held.release(chat).await;
                return Err(error);
            }
        },
        None => None,
    };
    let written = chat
        .rooms
        .reconfigure_guarded(&room.code, user_id, patch, Some(&held.guard()))
        .await;
    let undo = attached.filter(|_| written.is_err());
    tidy::tidy_after(chat, held, conversation, undo.as_ref(), None).await;
    Ok(written)
}

async fn attach(
    chat: &ChatState,
    held: &Held,
    conversation: Uuid,
    user_id: &str,
    media: Uuid,
) -> ChatResult<Attached> {
    let before = room_of_chat(chat, conversation)
        .await?
        .and_then(|room| room.image_backdrop());
    let mut tx = chat.pg.begin().await?;
    let upload = sqlx::query_file_as!(Upload, "queries/chat/backdrop/lock_media.sql", media)
        .fetch_optional(&mut *tx)
        .await?;
    usable(upload, Some(conversation), user_id, media)?;
    sqlx::query_file!("queries/chat/backdrop/insert_ref.sql", media, conversation)
        .execute(&mut *tx)
        .await?;
    let flipped = sqlx::query_file!("queries/chat/backdrop/attach.sql", media)
        .execute(&mut *tx)
        .await?
        .rows_affected()
        > 0;
    let meta = if before == Some(media) {
        None
    } else {
        sqlx::query_file!("queries/chat/backdrop/forget_meta.sql", conversation)
            .fetch_optional(&mut *tx)
            .await?
            .and_then(|old| old.meta_ct.zip(old.meta_epoch))
    };
    if !held.kept(chat).await {
        return Err(ChatError::busy());
    }
    tx.commit().await?;
    Ok(Attached {
        media,
        before,
        flipped,
        meta,
    })
}

pub async fn clear(
    chat: &ChatState,
    conversation: Uuid,
    media: Uuid,
    cooldown_until: Option<i64>,
) -> ChatResult<bool> {
    let held = Held::take(chat, conversation).await?;
    let cleared = match room_of_chat(chat, conversation).await {
        Ok(Some(room)) => chat
            .rooms
            .clear_image_backdrop(&room.code, media, cooldown_until, &held.guard())
            .await
            .map_err(refusal),
        Ok(None) => Ok(false),
        Err(error) => Err(error),
    };
    tidy::tidy_after(chat, held, conversation, None, Some(media)).await;
    cleared
}
