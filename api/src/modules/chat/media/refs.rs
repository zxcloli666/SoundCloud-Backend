use sc_chat_wire::msg::Class;
use sqlx::{PgConnection, PgExecutor};
use uuid::Uuid;

use crate::modules::chat::media::ledger;
use crate::modules::chat::media::limits::ROOM_ALBUM_MAX;
use crate::modules::chat::members::access::Access;
use crate::modules::chat::messages::rules::Outgoing;
use crate::modules::chat::model::MessageRow;
use crate::modules::chat::rooms::media_gate;
use crate::modules::chat::{ChatError, ChatResult, ChatState};

pub async fn admit_send(chat: &ChatState, access: &Access, out: &Outgoing) -> ChatResult<()> {
    match out.class {
        Class::Meta if access.is_room() => {
            media_gate::backdrop_meta(chat, access.conversation, &access.user_id, &out.media).await
        }
        Class::Meta if !out.media.is_empty() => admit_avatar(chat, access, out.media[0]).await,
        Class::Post if !out.media.is_empty() => admit_post(chat, access, &out.media).await,
        _ if out.media.is_empty() => Ok(()),
        _ => Err(ChatError::Invalid),
    }
}

struct SendRow {
    id: Uuid,
    kind: String,
    status: String,
    own: bool,
    origin_conversation: Uuid,
    origin_room: bool,
    origin_known: bool,
    here: bool,
    elsewhere: bool,
}

impl SendRow {
    fn usable_in(&self, access: &Access) -> bool {
        let isolated = if self.origin_room || access.is_room() {
            self.origin_conversation == access.conversation
        } else {
            true
        };
        let placed = match self.status.as_str() {
            "attached" => true,
            "uploaded" => self.origin_known,
            _ => false,
        };
        self.kind == "image" && placed && (self.own || self.here || self.elsewhere) && isolated
    }
}

async fn usable(
    executor: impl PgExecutor<'_>,
    access: &Access,
    media: &[Uuid],
) -> ChatResult<Vec<Uuid>> {
    let rows = sqlx::query_file_as!(
        SendRow,
        "queries/chat/media/send_rows.sql",
        media,
        access.user_id,
        access.conversation,
        access.history_floor()
    )
    .fetch_all(executor)
    .await?;
    Ok(rows
        .into_iter()
        .filter(|row| row.usable_in(access))
        .map(|row| row.id)
        .collect())
}

async fn admit_post(chat: &ChatState, access: &Access, media: &[Uuid]) -> ChatResult<()> {
    if access.is_room() && media.len() > ROOM_ALBUM_MAX {
        return Err(ChatError::Invalid);
    }
    refuse_missing(media, &usable(&chat.pg, access, media).await?)?;
    if access.is_room() {
        media_gate::image_gate(chat, access.conversation, &access.user_id).await?;
    }
    Ok(())
}

async fn admit_avatar(chat: &ChatState, access: &Access, id: Uuid) -> ChatResult<()> {
    let row = sqlx::query_file!(
        "queries/chat/media/avatar_row.sql",
        id,
        access.user_id,
        access.conversation
    )
    .fetch_optional(&chat.pg)
    .await?;
    let usable = row.is_some_and(|row| {
        row.kind == "avatar"
            && matches!(row.status.as_str(), "uploaded" | "attached")
            && (row.current || (row.own && row.origin_here))
    });
    if usable {
        Ok(())
    } else {
        Err(ChatError::MediaMissing { ids: vec![id] })
    }
}

fn refuse_missing(asked: &[Uuid], usable: &[Uuid]) -> ChatResult<()> {
    let missing: Vec<Uuid> = asked
        .iter()
        .filter(|id| !usable.contains(id))
        .copied()
        .collect();
    if missing.is_empty() {
        Ok(())
    } else {
        Err(ChatError::MediaMissing { ids: missing })
    }
}

pub async fn attach(
    tx: &mut PgConnection,
    access: &Access,
    row: &MessageRow,
    media: &[Uuid],
) -> ChatResult<()> {
    match Class::parse(&row.class) {
        Some(Class::Post) if !media.is_empty() => {
            sqlx::query_file_scalar!("queries/chat/media/lock_sources.sql", media, access.user_id)
                .fetch_all(&mut *tx)
                .await?;
            lock_usable(tx, media, "image").await?;
            refuse_missing(media, &usable(&mut *tx, access, media).await?)?;
            add_refs(tx, media, row.conversation_id, row.seq).await
        }
        Some(Class::Meta) if access.is_group() => {
            replace_avatar(tx, row.conversation_id, media).await
        }
        _ => Ok(()),
    }
}

async fn replace_avatar(
    tx: &mut PgConnection,
    conversation: Uuid,
    media: &[Uuid],
) -> ChatResult<()> {
    let mut touched =
        sqlx::query_file_scalar!("queries/chat/media/avatar_current.sql", conversation)
            .fetch_all(&mut *tx)
            .await?;
    touched.extend_from_slice(media);
    ledger::lock(tx, &touched).await?;
    if !media.is_empty() {
        lock_usable(tx, media, "avatar").await?;
    }
    let dropped =
        sqlx::query_file_scalar!("queries/chat/media/avatar_drop.sql", conversation, media)
            .fetch_all(&mut *tx)
            .await?;
    if !media.is_empty() {
        add_refs(tx, media, conversation, 0).await?;
    }
    ledger::settle(tx, &dropped).await?;
    Ok(())
}

async fn lock_usable(tx: &mut PgConnection, media: &[Uuid], kind: &str) -> ChatResult<()> {
    let locked = sqlx::query_file!("queries/chat/media/attach_lock.sql", media)
        .fetch_all(&mut *tx)
        .await?;
    let usable: Vec<Uuid> = locked
        .into_iter()
        .filter(|row| row.kind == kind && matches!(row.status.as_str(), "uploaded" | "attached"))
        .map(|row| row.id)
        .collect();
    refuse_missing(media, &usable)
}

async fn add_refs(
    tx: &mut PgConnection,
    media: &[Uuid],
    conversation: Uuid,
    seq: i64,
) -> ChatResult<()> {
    sqlx::query_file!(
        "queries/chat/media/attach_refs.sql",
        media,
        conversation,
        seq
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query_file!("queries/chat/media/attach_flip.sql", media)
        .execute(&mut *tx)
        .await?;
    Ok(())
}

#[cfg(test)]
#[path = "../tests/media_send_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "../tests/media_forward_race_tests.rs"]
mod forward_race_tests;
