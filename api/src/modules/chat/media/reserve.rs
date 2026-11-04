use std::sync::Arc;

use axum::Json;
use axum::body::Bytes;
use axum::extract::State;
use uuid::Uuid;

use crate::modules::chat::limits::{self, Bucket};
use crate::modules::chat::media::ledger;
use crate::modules::chat::media::limits::{PUT_TTL, RESERVATION_TTL};
use crate::modules::chat::media::wire::{MediaKind, ReserveAnswer, ReserveBody, ReservedThumb};
use crate::modules::chat::members::access;
use crate::modules::chat::quota::{self, Used, seconds, signed, unsigned};
use crate::modules::chat::rooms::media_gate;
use crate::modules::chat::{ChatCaller, ChatError, ChatResult, ChatState, stores};

#[derive(Debug, Clone)]
struct Object {
    id: Uuid,
    key: String,
    bytes: u64,
}

#[derive(Debug, Clone)]
struct Planned {
    store: u16,
    root: Object,
    thumb: Option<Object>,
}

impl Planned {
    fn fresh(store: u16, body: &ReserveBody) -> Self {
        let object = |bytes| Object {
            id: Uuid::new_v4(),
            key: stores::new_key(),
            bytes,
        };
        Self {
            store,
            root: object(body.bytes),
            thumb: body.thumb_bytes.map(object),
        }
    }
}

pub async fn reserve_media(
    caller: ChatCaller,
    State(chat): State<Arc<ChatState>>,
    raw: Bytes,
) -> ChatResult<Json<ReserveAnswer>> {
    if !chat.config.media.uploads {
        return Err(ChatError::MediaUploadsPaused);
    }
    let body: ReserveBody = serde_json::from_slice(&raw).map_err(|_| ChatError::Invalid)?;
    body.validate()?;
    let user = caller.user_id.as_str();
    let matched = matched(&chat, user, &body).await?;
    if !chat.stars.is_star(user).await? {
        return Err(ChatError::MediaStarRequired);
    }
    admit_place(&chat, user, &body).await?;
    if let Some(planned) = matched
        && refreshed(&chat, &planned).await?
    {
        return replay(&chat, user, &planned).await.map(Json);
    }
    reserve_rows(&chat, user, &body).await.map(Json)
}

async fn matched(chat: &ChatState, user: &str, body: &ReserveBody) -> ChatResult<Option<Planned>> {
    let Some(row) = sqlx::query_file!("queries/chat/media/reserve_match.sql", user, body.upload_id)
        .fetch_optional(&chat.pg)
        .await?
    else {
        return Ok(None);
    };
    let thumb = match (row.thumb_id, row.thumb_key, row.thumb_bytes) {
        (Some(id), Some(key), Some(bytes)) => Some(Object {
            id,
            key,
            bytes: unsigned(bytes),
        }),
        _ => None,
    };
    let planned = Planned {
        store: u16::try_from(row.store).map_err(|_| ChatError::Internal)?,
        root: Object {
            id: row.id,
            key: row.object_key,
            bytes: unsigned(row.bytes),
        },
        thumb,
    };
    let same = MediaKind::parse(&row.kind) == Some(body.kind)
        && row.origin_conversation == body.conversation_id
        && planned.root.bytes == body.bytes
        && planned.thumb.as_ref().map(|thumb| thumb.bytes) == body.thumb_bytes;
    if same {
        Ok(Some(planned))
    } else {
        Err(ChatError::Invalid)
    }
}

async fn admit_place(chat: &ChatState, user: &str, body: &ReserveBody) -> ChatResult<()> {
    let conversation = body.conversation_id;
    match body.kind {
        MediaKind::Image => {
            let access = access::load_in(chat, conversation, user).await?.active()?;
            if access.is_room() {
                media_gate::image_gate(chat, conversation, user).await?;
            }
            Ok(())
        }
        MediaKind::Avatar => {
            let access = access::load_in(chat, conversation, user).await?;
            if !access.is_group() {
                return Err(ChatError::Invalid);
            }
            access.active()?.admin()?;
            Ok(())
        }
        MediaKind::Backdrop => media_gate::backdrop_gate(chat, conversation, user).await,
    }
}

async fn refreshed(chat: &ChatState, planned: &Planned) -> ChatResult<bool> {
    let mut tx = chat.pg.begin().await?;
    ledger::lock(&mut tx, &[planned.root.id]).await?;
    let touched = sqlx::query_file_scalar!(
        "queries/chat/media/reserve_refresh.sql",
        planned.root.id,
        seconds(RESERVATION_TTL)
    )
    .fetch_all(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(touched.contains(&planned.root.id))
}

async fn replay(chat: &ChatState, user: &str, planned: &Planned) -> ChatResult<ReserveAnswer> {
    let used = quota::used(chat, user).await?;
    answer(chat, planned, used).await
}

async fn reserve_rows(
    chat: &ChatState,
    user: &str,
    body: &ReserveBody,
) -> ChatResult<ReserveAnswer> {
    limits::admit(chat, Bucket::MediaReserve, user).await?;
    let store = stores::open_for_writes(chat)
        .await?
        .ok_or(ChatError::MediaUploadsPaused)?;
    let planned = Planned::fresh(store, body);
    let mut tx = chat.pg.begin().await?;
    let live = sqlx::query_file_scalar!(
        "queries/chat/media/reserve_origin.sql",
        body.conversation_id
    )
    .fetch_optional(&mut *tx)
    .await?;
    if live != Some(true) {
        tx.rollback().await?;
        return Err(ChatError::NotMember);
    }
    let thumb = planned.thumb.as_ref();
    let inserted = sqlx::query_file!(
        "queries/chat/media/reserve_insert.sql",
        planned.root.id,
        i16::try_from(store).map_err(|_| ChatError::Internal)?,
        planned.root.key,
        user,
        body.conversation_id,
        body.kind.as_str(),
        body.upload_id,
        signed(planned.root.bytes),
        thumb.map(|thumb| thumb.id),
        thumb.map(|thumb| thumb.key.clone()),
        thumb.map(|thumb| signed(thumb.bytes)),
        seconds(RESERVATION_TTL)
    )
    .fetch_one(&mut *tx)
    .await?;
    if inserted.roots == 0 {
        tx.rollback().await?;
        let raced = matched(chat, user, body)
            .await?
            .ok_or_else(ChatError::busy)?;
        return replay(chat, user, &raced).await;
    }
    let used = quota::reserve(&mut tx, user, body.total()).await?;
    tx.commit().await?;
    answer(chat, &planned, used).await
}

async fn answer(chat: &ChatState, planned: &Planned, used: Used) -> ChatResult<ReserveAnswer> {
    let put = chat
        .s3
        .presign_put(
            planned.store,
            &planned.root.key,
            planned.root.bytes,
            PUT_TTL,
        )
        .await?;
    let thumb = match &planned.thumb {
        Some(thumb) => Some(ReservedThumb {
            id: thumb.id,
            put: chat
                .s3
                .presign_put(planned.store, &thumb.key, thumb.bytes, PUT_TTL)
                .await?,
        }),
        None => None,
    };
    Ok(ReserveAnswer {
        id: planned.root.id,
        put,
        thumb,
        expires_in_s: u32::try_from(PUT_TTL.as_secs()).unwrap_or(u32::MAX),
        used: used.total(),
        quota: quota::quota(),
    })
}

#[cfg(test)]
#[path = "../tests/media_reserve_tests.rs"]
mod tests;
