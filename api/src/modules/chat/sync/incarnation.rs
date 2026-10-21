use uuid::Uuid;

use crate::modules::chat::{ChatError, ChatResult, ChatState};

pub async fn current(chat: &ChatState) -> ChatResult<Uuid> {
    chat.hub
        .incarnation
        .get_or_try_init(|| async {
            sqlx::query_file_scalar!("queries/chat/sync/incarnation.sql")
                .fetch_one(&chat.pg)
                .await
                .map_err(ChatError::from)
        })
        .await
        .copied()
}
