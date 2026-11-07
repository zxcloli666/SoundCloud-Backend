use serde_json::Value;
use sqlx::PgConnection;
use uuid::Uuid;

use super::Chosen;
use crate::modules::chat::media::{backdrop, ledger};
use crate::modules::chat::members::{access::Access, roles};
use crate::modules::chat::{ChatError, ChatResult, ChatState};

struct Locked {
    kind: String,
    settings: Value,
    last_seq: i64,
    live: bool,
}

pub async fn apply(chat: &ChatState, access: &Access, chosen: Option<&Chosen>) -> ChatResult<()> {
    let conversation = access.conversation;
    let mut tx = chat.pg.begin().await?;
    let locked = lock(&mut tx, conversation)
        .await?
        .filter(|locked| locked.live)
        .ok_or(ChatError::NotFound)?;
    let before = Chosen::of(&locked.settings).and_then(|chosen| chosen.image());
    let after = chosen.and_then(Chosen::image);
    if let Some(media) = after
        && before != Some(media)
    {
        backdrop::attach_in(&mut tx, conversation, &access.user_id, media).await?;
    }
    match chosen {
        Some(chosen) => {
            sqlx::query_file!(
                "queries/chat/backdrops/set.sql",
                conversation,
                chosen.json()
            )
            .execute(&mut *tx)
            .await?;
        }
        None => {
            sqlx::query_file!("queries/chat/backdrops/clear.sql", conversation)
                .execute(&mut *tx)
                .await?;
        }
    }
    if let Some(old) = before
        && after != Some(old)
    {
        withdraw(&mut tx, conversation, old).await?;
    }
    tx.commit().await?;
    Ok(())
}

pub async fn clear_image(chat: &ChatState, conversation: Uuid, media: Uuid) -> ChatResult<bool> {
    let mut tx = chat.pg.begin().await?;
    let Some(locked) = lock(&mut tx, conversation).await? else {
        tx.rollback().await?;
        return Ok(false);
    };
    withdraw(&mut tx, conversation, media).await?;
    let shown = Chosen::of(&locked.settings).and_then(|chosen| chosen.image()) == Some(media);
    if !shown {
        tx.commit().await?;
        return Ok(false);
    }
    sqlx::query_file!("queries/chat/backdrops/clear.sql", conversation)
        .execute(&mut *tx)
        .await?;
    let recipients =
        sqlx::query_file_scalar!("queries/chat/backdrops/recipients.sql", conversation)
            .fetch_all(&mut *tx)
            .await?;
    tx.commit().await?;
    let kind = roles::kind(&locked.kind)?;
    super::notify(chat, kind, conversation, locked.last_seq, &recipients).await?;
    Ok(true)
}

pub async fn image_in(tx: &mut PgConnection, conversation: Uuid) -> ChatResult<Option<Uuid>> {
    let settings = sqlx::query_file_scalar!("queries/chat/backdrops/settings_of.sql", conversation)
        .fetch_optional(&mut *tx)
        .await?;
    Ok(settings
        .as_ref()
        .and_then(Chosen::of)
        .and_then(|chosen| chosen.image()))
}

async fn lock(tx: &mut PgConnection, conversation: Uuid) -> ChatResult<Option<Locked>> {
    Ok(
        sqlx::query_file_as!(Locked, "queries/chat/backdrops/lock.sql", conversation)
            .fetch_optional(&mut *tx)
            .await?,
    )
}

async fn withdraw(tx: &mut PgConnection, conversation: Uuid, media: Uuid) -> ChatResult<()> {
    let dropped = sqlx::query_file_scalar!(
        "queries/chat/backdrop/withdraw_ref.sql",
        media,
        conversation
    )
    .fetch_all(&mut *tx)
    .await?;
    ledger::settle(tx, &dropped).await?;
    Ok(())
}
