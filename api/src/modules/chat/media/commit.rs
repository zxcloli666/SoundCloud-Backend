use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use futures::future::{join_all, try_join_all};
use uuid::Uuid;

use crate::modules::chat::body::ChatPath;
use crate::modules::chat::errors::BusySource;
use crate::modules::chat::media::ledger;
use crate::modules::chat::media::limits::QUOTA_BYTES;
use crate::modules::chat::media::wire::CommitAnswer;
use crate::modules::chat::quota::{self, unsigned};
use crate::modules::chat::stores::{self, Object};
use crate::modules::chat::{ChatCaller, ChatError, ChatResult, ChatState};

pub const LEFT_FOR_RECONCILE: &str = "chat_media_commit_leftovers_total";

pub async fn commit_media(
    caller: ChatCaller,
    State(chat): State<Arc<ChatState>>,
    ChatPath(id): ChatPath<Uuid>,
) -> ChatResult<Json<CommitAnswer>> {
    commit_rows(&chat, &caller, id).await.map(Json)
}

async fn commit_rows(chat: &ChatState, caller: &ChatCaller, id: Uuid) -> ChatResult<CommitAnswer> {
    let user = caller.user_id.as_str();
    let row = sqlx::query_file!("queries/chat/media/commit_row.sql", id, user)
        .fetch_optional(&chat.pg)
        .await?
        .ok_or(ChatError::NotFound)?;
    let mut objects = vec![Object {
        id: row.id,
        key: row.object_key,
        bytes: unsigned(row.bytes),
    }];
    if let (Some(id), Some(key), Some(bytes)) = (row.thumb_id, row.thumb_key, row.thumb_bytes) {
        objects.push(Object {
            id,
            key,
            bytes: unsigned(bytes),
        });
    }
    let total = objects.iter().map(|object| object.bytes).sum();
    match row.status.as_str() {
        "uploaded" | "attached" => return answer(chat, user, id, total).await,
        "reserved" if row.live => {}
        _ => return Err(ChatError::NotFound),
    }
    let store = u16::try_from(row.store).map_err(|_| ChatError::Internal)?;
    let missing = missing(chat, store, &objects).await?;
    if !missing.is_empty() {
        return committed(chat, user, id, total)
            .await?
            .ok_or(ChatError::MediaMissing { ids: missing });
    }
    let Some(kept) = kept_copies(chat, store, &objects).await? else {
        return committed(chat, user, id, total)
            .await?
            .ok_or(ChatError::NotFound);
    };
    let flipped = match ledger::commit(&chat.pg, user, id, &kept).await {
        Ok(flipped) => flipped,
        Err(error) => {
            discard(chat, store, &kept).await;
            return Err(error);
        }
    };
    match flipped {
        Some(used) => {
            discard(chat, store, &objects).await;
            Ok(CommitAnswer {
                id,
                bytes: total,
                used: used.total(),
                quota: QUOTA_BYTES,
            })
        }
        None => {
            discard(chat, store, &kept).await;
            committed(chat, user, id, total)
                .await?
                .ok_or(ChatError::NotFound)
        }
    }
}

async fn missing(chat: &ChatState, store: u16, objects: &[Object]) -> ChatResult<Vec<Uuid>> {
    let heads = try_join_all(
        objects
            .iter()
            .map(|object| chat.s3.head(store, &object.key)),
    )
    .await?;
    Ok(objects
        .iter()
        .zip(heads)
        .filter(|(object, length)| *length != Some(object.bytes))
        .map(|(object, _)| object.id)
        .collect())
}

async fn kept_copies(
    chat: &ChatState,
    store: u16,
    uploads: &[Object],
) -> ChatResult<Option<Vec<Object>>> {
    let kept: Vec<Object> = uploads
        .iter()
        .map(|upload| Object {
            key: stores::new_key(),
            ..upload.clone()
        })
        .collect();
    let copied = join_all(
        uploads
            .iter()
            .zip(&kept)
            .map(|(upload, copy)| chat.s3.copy(store, &upload.key, &copy.key)),
    )
    .await;
    if copied.iter().all(|done| matches!(done, Ok(true))) {
        return match missing(chat, store, &kept).await {
            Ok(short) if short.is_empty() => Ok(Some(kept)),
            verified => {
                discard(chat, store, &kept).await;
                verified.and(Err(ChatError::busy(BusySource::S3)))
            }
        };
    }
    let made: Vec<Object> = kept
        .into_iter()
        .zip(&copied)
        .filter(|(_, done)| matches!(done, Ok(true)))
        .map(|(copy, _)| copy)
        .collect();
    discard(chat, store, &made).await;
    match copied.into_iter().find_map(Result::err) {
        Some(error) => Err(error),
        None => Ok(None),
    }
}

async fn discard(chat: &ChatState, store: u16, objects: &[Object]) {
    let deleted = join_all(
        objects
            .iter()
            .map(|object| chat.s3.delete(store, &object.key)),
    )
    .await;
    for (object, done) in objects.iter().zip(deleted) {
        if let Err(error) = done {
            metrics::counter!(LEFT_FOR_RECONCILE).increment(1);
            tracing::warn!(
                store,
                key = %object.key,
                code = error.code(),
                "a chat media object waits for the nightly reconcile"
            );
        }
    }
}

async fn committed(
    chat: &ChatState,
    user: &str,
    id: Uuid,
    total: u64,
) -> ChatResult<Option<CommitAnswer>> {
    let status = sqlx::query_file!("queries/chat/media/commit_row.sql", id, user)
        .fetch_optional(&chat.pg)
        .await?
        .map(|row| row.status);
    match status.as_deref() {
        Some("uploaded" | "attached") => answer(chat, user, id, total).await.map(Some),
        _ => Ok(None),
    }
}

async fn answer(chat: &ChatState, user: &str, id: Uuid, total: u64) -> ChatResult<CommitAnswer> {
    Ok(CommitAnswer {
        id,
        bytes: total,
        used: quota::used(chat, user).await?.total(),
        quota: QUOTA_BYTES,
    })
}

#[cfg(test)]
#[path = "../tests/media_commit_tests.rs"]
mod tests;
