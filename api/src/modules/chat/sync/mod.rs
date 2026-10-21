pub mod account;
pub mod answer;
pub mod bucket;
pub mod bump;
pub mod bytes;
pub mod changes;
pub mod incarnation;
pub mod marks;
pub mod plan;
pub mod poll;
pub mod prefs;
pub mod reply;
pub mod rows;
pub mod seats;
pub mod shards;
pub mod wire;

use std::sync::Arc;

use axum::extract::State;

use crate::modules::chat::body::ChatJson;
use crate::modules::chat::sync::reply::SyncReply;
use crate::modules::chat::sync::wire::SyncRequest;
use crate::modules::chat::{ChatCaller, ChatError, ChatResult, ChatState, presence};

pub async fn sync(
    caller: ChatCaller,
    State(chat): State<Arc<ChatState>>,
    ChatJson(request): ChatJson<SyncRequest>,
) -> ChatResult<SyncReply> {
    let device = caller
        .device
        .as_ref()
        .map(|device| device.id)
        .ok_or(ChatError::Invalid)?;
    request.validate()?;
    marks::record(&chat, &caller.user_id, &request).await?;
    presence::touch(&chat, &caller.user_id).await;
    poll::run(&chat, &caller, device, &request).await
}

#[cfg(test)]
#[path = "../tests/sync_tests.rs"]
mod tests;
