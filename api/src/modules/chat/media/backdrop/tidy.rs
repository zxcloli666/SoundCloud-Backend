use uuid::Uuid;

use super::lock::Held;
use crate::modules::chat::media::ledger;
use crate::modules::chat::rooms::lookup::room_of_chat;
use crate::modules::chat::{ChatResult, ChatState};

pub struct Attached {
    pub media: Uuid,
    pub before: Option<Uuid>,
    pub flipped: bool,
    pub meta: Option<(Vec<u8>, i32)>,
}

pub(super) async fn tidy_after(
    chat: &ChatState,
    held: Held,
    conversation: Uuid,
    undo: Option<&Attached>,
    released: Option<Uuid>,
) {
    let first = tidy(chat, &held, conversation, undo, released).await;
    held.release(chat).await;
    let tidied = match first {
        Ok(true) => Ok(true),
        _ => tidy_again(chat, conversation, undo, released).await,
    };
    match tidied {
        Ok(true) => {}
        Ok(false) => tracing::warn!(%conversation, "room backdrop refs wait for the next change"),
        Err(error) => tracing::warn!(
            %conversation,
            code = error.code(),
            "room backdrop refs wait for the next change"
        ),
    }
}

async fn tidy_again(
    chat: &ChatState,
    conversation: Uuid,
    undo: Option<&Attached>,
    released: Option<Uuid>,
) -> ChatResult<bool> {
    let held = Held::take(chat, conversation).await?;
    let tidied = tidy(chat, &held, conversation, undo, released).await;
    held.release(chat).await;
    tidied
}

pub async fn tidy(
    chat: &ChatState,
    held: &Held,
    conversation: Uuid,
    undo: Option<&Attached>,
    released: Option<Uuid>,
) -> ChatResult<bool> {
    let Some(room) = room_of_chat(chat, conversation).await? else {
        return Ok(true);
    };
    let stored = room.image_backdrop();
    let mut tx = chat.pg.begin().await?;
    if let Some(attached) = undo.filter(|attached| stored != Some(attached.media)) {
        if attached.flipped {
            sqlx::query_file!(
                "queries/chat/backdrop/undo_attach.sql",
                attached.media,
                conversation
            )
            .execute(&mut *tx)
            .await?;
        }
        if let Some((body, epoch)) = attached.meta.as_ref().filter(|_| stored == attached.before) {
            sqlx::query_file!(
                "queries/chat/backdrop/restore_meta.sql",
                conversation,
                body.as_slice(),
                *epoch
            )
            .execute(&mut *tx)
            .await?;
        }
    }
    let shown: Vec<Uuid> = stored.into_iter().collect();
    let released: Vec<Uuid> = released.into_iter().collect();
    let dropped = sqlx::query_file_scalar!(
        "queries/chat/backdrop/drop_refs.sql",
        conversation,
        &shown,
        room.host_id,
        &released
    )
    .fetch_all(&mut *tx)
    .await?;
    ledger::settle(&mut tx, &dropped).await?;
    if !held.kept(chat).await {
        return Ok(false);
    }
    tx.commit().await?;
    Ok(true)
}
