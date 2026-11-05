pub mod lock;
mod tidy;

use uuid::Uuid;

use crate::modules::chat::media::ledger;
use crate::modules::chat::rooms::lookup::room_of_chat;
use crate::modules::chat::rooms::refusal;
use crate::modules::chat::{ChatError, ChatResult, ChatState};
use crate::modules::rooms::model::{Room, SettingsPatch};

use lock::Held;
#[cfg(test)]
pub use tidy::tidy;
pub use tidy::{Attached, Settle};

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
) -> ChatResult<Room> {
    let Some(conversation) = room.chat_id else {
        if let Some(media) = patch.wanted_image() {
            return Err(ChatError::MediaMissing { ids: vec![media] });
        }
        return chat
            .rooms
            .reconfigure(&room.code, user_id, patch)
            .await
            .map_err(refusal);
    };
    let held = Held::take(chat, conversation).await?;
    let prepared = prepare(chat, conversation, user_id, patch).await;
    let (fence, attached) = match prepared {
        Ok(prepared) => prepared,
        Err(error) => {
            held.release(chat).await;
            return Err(error);
        }
    };
    let written = chat
        .rooms
        .reconfigure_guarded(&room.code, user_id, patch, None, Some(&held.guard()))
        .await;
    let settle = Settle {
        fence,
        attached,
        written: written.is_ok(),
        replaced: written.as_ref().ok().and_then(|(_, replaced)| *replaced),
        released: None,
    };
    tidy::tidy_after(chat, held, conversation, settle).await;
    written.map(|(room, _)| room).map_err(refusal)
}

async fn prepare(
    chat: &ChatState,
    conversation: Uuid,
    user_id: &str,
    patch: &SettingsPatch,
) -> ChatResult<(Option<Uuid>, Option<Attached>)> {
    let Some(media) = patch.wanted_image() else {
        return Ok((fence_of(chat, conversation).await?, None));
    };
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
    let (fence, attached) = attach(chat, conversation, user_id, media).await?;
    Ok((Some(fence), Some(attached)))
}

async fn fence_of(chat: &ChatState, conversation: Uuid) -> ChatResult<Option<Uuid>> {
    Ok(
        sqlx::query_file_scalar!("queries/chat/backdrop/fence_of.sql", conversation)
            .fetch_optional(&chat.pg)
            .await?
            .flatten(),
    )
}

pub async fn attach(
    chat: &ChatState,
    conversation: Uuid,
    user_id: &str,
    media: Uuid,
) -> ChatResult<(Uuid, Attached)> {
    let fence = Uuid::new_v4();
    let mut tx = chat.pg.begin().await?;
    let live = sqlx::query_file_scalar!("queries/chat/backdrop/fence.sql", conversation, fence)
        .fetch_optional(&mut *tx)
        .await?;
    if live.is_none() {
        return Err(ChatError::MediaMissing { ids: vec![media] });
    }
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
    tx.commit().await?;
    Ok((fence, Attached { media, flipped }))
}

pub async fn clear(
    chat: &ChatState,
    conversation: Uuid,
    media: Uuid,
    cooldown_until: Option<i64>,
) -> ChatResult<bool> {
    let held = Held::take(chat, conversation).await?;
    let cleared = clear_held(chat, &held, conversation, media, cooldown_until).await;
    let settle = Settle {
        fence: cleared.as_ref().ok().and_then(|(fence, _)| *fence),
        released: Some(media),
        ..Settle::default()
    };
    if cleared.is_ok() {
        tidy::tidy_after(chat, held, conversation, settle).await;
    } else {
        held.release(chat).await;
    }
    cleared.map(|(_, cleared)| cleared)
}

async fn clear_held(
    chat: &ChatState,
    held: &Held,
    conversation: Uuid,
    media: Uuid,
    cooldown_until: Option<i64>,
) -> ChatResult<(Option<Uuid>, bool)> {
    withdraw(chat, conversation, media).await?;
    let fence = fence_of(chat, conversation).await?;
    let Some(room) = room_of_chat(chat, conversation).await? else {
        return Ok((fence, false));
    };
    let cleared = chat
        .rooms
        .clear_image_backdrop(&room.code, media, &held.guard())
        .await
        .map_err(refusal)?;
    let Some(after) = cleared else {
        return Ok((fence, false));
    };
    if let Some(until) = cooldown_until {
        let cooled = chat.rooms.cool_down_backdrop(&after.host_id, until).await;
        if let Err(error) = cooled {
            tracing::warn!(%conversation, %error, "a cleared room backdrop has no host cooldown");
        }
    }
    Ok((fence, true))
}

async fn withdraw(chat: &ChatState, conversation: Uuid, media: Uuid) -> ChatResult<()> {
    let mut tx = chat.pg.begin().await?;
    sqlx::query_file_scalar!("queries/chat/backdrop/lock_room.sql", conversation)
        .fetch_optional(&mut *tx)
        .await?;
    let dropped = sqlx::query_file_scalar!(
        "queries/chat/backdrop/withdraw_ref.sql",
        media,
        conversation
    )
    .fetch_all(&mut *tx)
    .await?;
    ledger::settle(&mut tx, &dropped).await?;
    tx.commit().await?;
    Ok(())
}
