use sc_chat_wire::msg::Class;
use sqlx::{PgConnection, PgExecutor};
use uuid::Uuid;

use crate::modules::chat::backdrops::{self, Chosen};
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
        Class::Meta => admit_meta(chat, access, &out.media).await,
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
    origin_isolated: bool,
    origin_known: bool,
    here: bool,
    elsewhere: bool,
}

impl SendRow {
    fn usable_in(&self, access: &Access) -> bool {
        let isolated = if self.origin_isolated || access.isolates_media() {
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
    if access.isolates_media() && media.len() > ROOM_ALBUM_MAX {
        return Err(ChatError::Invalid);
    }
    refuse_missing(media, &usable(&chat.pg, access, media).await?)?;
    if access.is_room() {
        media_gate::image_gate(chat, access.conversation, &access.user_id).await?;
    }
    Ok(())
}

struct MetaRow {
    id: Uuid,
    kind: String,
    status: String,
    own: bool,
    origin_here: bool,
    current: bool,
}

impl MetaRow {
    fn usable_in(&self, access: &Access) -> bool {
        let placed = matches!(self.status.as_str(), "uploaded" | "attached");
        match self.kind.as_str() {
            "avatar" => {
                placed && access.is_group() && (self.current || (self.own && self.origin_here))
            }
            "backdrop" => {
                self.current && access.backdrop.as_ref().and_then(Chosen::image) == Some(self.id)
            }
            _ => false,
        }
    }
}

async fn admit_meta(chat: &ChatState, access: &Access, media: &[Uuid]) -> ChatResult<()> {
    if media.is_empty() {
        return if access.is_group() {
            Ok(())
        } else {
            Err(ChatError::Invalid)
        };
    }
    let rows = sqlx::query_file_as!(
        MetaRow,
        "queries/chat/backdrops/meta_rows.sql",
        media,
        access.user_id,
        access.conversation
    )
    .fetch_all(&chat.pg)
    .await?;
    let usable: Vec<Uuid> = rows
        .iter()
        .filter(|row| row.usable_in(access))
        .map(|row| row.id)
        .collect();
    refuse_missing(media, &usable)?;
    let avatars = rows.iter().filter(|row| row.kind == "avatar").count();
    let backdrops = rows.iter().filter(|row| row.kind == "backdrop").count();
    let fits =
        avatars <= 1 && backdrops <= 1 && (access.is_group() || (avatars == 0 && backdrops == 1));
    if fits {
        Ok(())
    } else {
        Err(ChatError::Invalid)
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
        Some(Class::Meta) if !access.is_room() => {
            attach_meta(tx, access, row.conversation_id, media).await
        }
        _ => Ok(()),
    }
}

async fn attach_meta(
    tx: &mut PgConnection,
    access: &Access,
    conversation: Uuid,
    media: &[Uuid],
) -> ChatResult<()> {
    let locked = sqlx::query_file!("queries/chat/media/attach_lock.sql", media)
        .fetch_all(&mut *tx)
        .await?;
    let of_kind = |kind: &str| -> Vec<Uuid> {
        locked
            .iter()
            .filter(|row| row.kind == kind)
            .map(|row| row.id)
            .collect()
    };
    let avatars = of_kind("avatar");
    let backdrops = of_kind("backdrop");
    if access.is_group() {
        replace_avatar(tx, conversation, &avatars).await?;
    }
    if backdrops.is_empty() {
        return Ok(());
    }
    let live: Vec<Uuid> = backdrops::image_in(tx, conversation)
        .await?
        .into_iter()
        .collect();
    refuse_missing(&backdrops, &live)
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
