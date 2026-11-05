use uuid::Uuid;

use super::lock::Held;
use crate::modules::chat::media::ledger;
use crate::modules::chat::rooms::lookup::room_of_chat;
use crate::modules::chat::{ChatResult, ChatState};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Attached {
    pub media: Uuid,
    pub flipped: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Settle {
    pub fence: Option<Uuid>,
    pub attached: Option<Attached>,
    pub written: bool,
    pub replaced: Option<Uuid>,
    pub released: Option<Uuid>,
}

pub(super) async fn tidy_after(chat: &ChatState, held: Held, conversation: Uuid, settle: Settle) {
    let first = tidy(chat, conversation, settle).await;
    held.release(chat).await;
    if first.is_ok() {
        return;
    }
    if let Err(error) = tidy(chat, conversation, settle).await {
        tracing::warn!(
            %conversation,
            code = error.code(),
            "room backdrop refs wait for the next change"
        );
    }
}

pub async fn tidy(chat: &ChatState, conversation: Uuid, settle: Settle) -> ChatResult<()> {
    let Some(room) = room_of_chat(chat, conversation).await? else {
        return Ok(());
    };
    let stored = room.image_backdrop();
    let mut tx = chat.pg.begin().await?;
    let fenced = sqlx::query_file_scalar!(
        "queries/chat/backdrop/fence_lock.sql",
        conversation,
        settle.fence
    )
    .fetch_optional(&mut *tx)
    .await?;
    if fenced.is_none() {
        return Ok(());
    }
    if let Some(attached) = settle.attached {
        if stored == Some(attached.media) {
            sqlx::query_file!("queries/chat/backdrop/clear_meta.sql", conversation)
                .execute(&mut *tx)
                .await?;
        } else if attached.flipped && !settle.written {
            sqlx::query_file!(
                "queries/chat/backdrop/undo_attach.sql",
                attached.media,
                conversation
            )
            .execute(&mut *tx)
            .await?;
        }
    }
    let shown: Vec<Uuid> = stored.into_iter().collect();
    let released: Vec<Uuid> = settle.replaced.into_iter().chain(settle.released).collect();
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
    tx.commit().await?;
    Ok(())
}
