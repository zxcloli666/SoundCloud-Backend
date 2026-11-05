use std::sync::Arc;

use axum::extract::State;
use axum::http::StatusCode;
use serde_json::json;
use uuid::Uuid;

use crate::modules::chat::admin::audit;
use crate::modules::chat::admin::auth::ChatAdmin;
use crate::modules::chat::body::ChatPath;
use crate::modules::chat::media;
use crate::modules::chat::moderation::actions;
use crate::modules::chat::{ChatError, ChatResult, ChatState};

pub async fn delete(
    admin: ChatAdmin,
    State(chat): State<Arc<ChatState>>,
    ChatPath((conversation, seq)): ChatPath<(Uuid, i64)>,
) -> ChatResult<StatusCode> {
    let mut tx = chat.pg.begin().await?;
    let removed =
        sqlx::query_file_scalar!("queries/chat/admin/delete_message.sql", conversation, seq)
            .fetch_all(&mut *tx)
            .await?;
    if removed.is_empty() {
        return Err(ChatError::NotFound);
    }
    media::hooks::drop_for_messages(&mut tx, conversation, &removed).await?;
    tx.commit().await?;
    chat.hot.forget(conversation);
    audit::record(
        &chat,
        &admin.admin,
        "delete_message",
        &audit::message(conversation, seq),
        json!({"seqs": removed}),
    )
    .await?;
    actions::announce_removed(&chat, conversation, &removed).await;
    Ok(StatusCode::NO_CONTENT)
}
