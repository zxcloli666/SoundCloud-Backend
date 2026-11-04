use std::sync::Arc;

use axum::Json;
use axum::extract::State;

use crate::modules::chat::body::ChatJson;
use crate::modules::chat::limits::{self, Bucket};
use crate::modules::chat::media::wire::{DeleteAnswer, Picked, Selection};
use crate::modules::chat::quota;
use crate::modules::chat::{ChatCaller, ChatResult, ChatState};

pub async fn delete_media(
    caller: ChatCaller,
    State(chat): State<Arc<ChatState>>,
    ChatJson(selection): ChatJson<Selection>,
) -> ChatResult<Json<DeleteAnswer>> {
    limits::admit(&chat, Bucket::MediaDelete, &caller.user_id).await?;
    let picked = selection.pick()?;
    delete_rows(&chat, &caller, &picked).await.map(Json)
}

async fn delete_rows(
    chat: &ChatState,
    caller: &ChatCaller,
    picked: &Picked,
) -> ChatResult<DeleteAnswer> {
    let (released, used) = quota::release(&chat.pg, &caller.user_id, picked).await?;
    Ok(DeleteAnswer {
        deleted: released.roots,
        freed: released.freed,
        used: used.total(),
        quota: quota::quota(),
    })
}

#[cfg(test)]
#[path = "../tests/media_delete_tests.rs"]
mod tests;
