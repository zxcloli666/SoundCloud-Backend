use std::sync::Arc;

use axum::extract::State;
use axum::http::StatusCode;
use serde_json::json;

use crate::modules::chat::admin::audit;
use crate::modules::chat::admin::auth::ChatAdmin;
use crate::modules::chat::body::ChatPath;
use crate::modules::chat::{ChatError, ChatResult, ChatState};
use crate::modules::rooms::store::RoomStore;

pub async fn unlist(
    admin: ChatAdmin,
    State(chat): State<Arc<ChatState>>,
    ChatPath(code): ChatPath<String>,
) -> ChatResult<StatusCode> {
    let room = RoomStore::new(chat.redis.clone())
        .load(&code)
        .await
        .map_err(|_| ChatError::Busy { retry_ms: 1_000 })?
        .ok_or(ChatError::NotFound)?;
    if !chat
        .rooms
        .unlist(&code)
        .await
        .map_err(|_| ChatError::Busy { retry_ms: 1_000 })?
    {
        return Err(ChatError::NotFound);
    }
    audit::record(
        &chat,
        &admin.admin,
        "unlist",
        &audit::room(&code),
        json!({"host": room.host_id, "was_public": room.public, "was_unlisted": room.unlisted}),
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}
