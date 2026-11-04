use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use futures::future::try_join_all;
use uuid::Uuid;

use crate::modules::chat::body::ChatPath;
use crate::modules::chat::media::wire::CommitAnswer;
use crate::modules::chat::quota::{self, unsigned};
use crate::modules::chat::{ChatCaller, ChatError, ChatResult, ChatState};

struct Stored {
    id: Uuid,
    key: String,
    bytes: u64,
}

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
    let mut objects = vec![Stored {
        id: row.id,
        key: row.object_key,
        bytes: unsigned(row.bytes),
    }];
    if let (Some(id), Some(key), Some(bytes)) = (row.thumb_id, row.thumb_key, row.thumb_bytes) {
        objects.push(Stored {
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
        return Err(ChatError::MediaMissing { ids: missing });
    }
    match quota::commit(&chat.pg, user, id).await? {
        Some(used) => Ok(CommitAnswer {
            id,
            bytes: total,
            used: used.total(),
            quota: quota::quota(),
        }),
        None => settled(chat, user, id, total).await,
    }
}

async fn missing(chat: &ChatState, store: u16, objects: &[Stored]) -> ChatResult<Vec<Uuid>> {
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

async fn settled(chat: &ChatState, user: &str, id: Uuid, total: u64) -> ChatResult<CommitAnswer> {
    let status = sqlx::query_file!("queries/chat/media/commit_row.sql", id, user)
        .fetch_optional(&chat.pg)
        .await?
        .map(|row| row.status);
    match status.as_deref() {
        Some("uploaded" | "attached") => answer(chat, user, id, total).await,
        _ => Err(ChatError::NotFound),
    }
}

async fn answer(chat: &ChatState, user: &str, id: Uuid, total: u64) -> ChatResult<CommitAnswer> {
    Ok(CommitAnswer {
        id,
        bytes: total,
        used: quota::used(chat, user).await?.total(),
        quota: quota::quota(),
    })
}

#[cfg(test)]
#[path = "../tests/media_commit_tests.rs"]
mod tests;
