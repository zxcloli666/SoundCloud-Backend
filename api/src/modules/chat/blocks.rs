use std::sync::Arc;

use axum::extract::State;
use axum::http::StatusCode;
use serde::Serialize;

use crate::modules::chat::body::ChatPath;
use crate::modules::chat::{ChatCaller, ChatError, ChatResult, ChatState, users};

pub const BLOCKS_MAX: i64 = 1_000;

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct BlockedView {
    pub user_id: String,
    pub username: Option<String>,
    pub avatar_url: Option<String>,
    pub at: i64,
}

pub async fn block(
    caller: ChatCaller,
    State(chat): State<Arc<ChatState>>,
    ChatPath(user): ChatPath<String>,
) -> ChatResult<StatusCode> {
    if !users::valid_user_id(&user) {
        return Err(ChatError::NotFound);
    }
    if user == caller.user_id {
        return Err(ChatError::Invalid);
    }
    let inserted = sqlx::query_file_scalar!(
        "queries/chat/blocks/insert.sql",
        caller.user_id,
        user,
        BLOCKS_MAX
    )
    .fetch_optional(&chat.pg)
    .await?;
    if inserted.is_none() && !blocked_by(&chat, &caller.user_id, &user).await? {
        return Err(ChatError::RequestLimit);
    }
    Ok(StatusCode::NO_CONTENT)
}

pub async fn unblock(
    caller: ChatCaller,
    State(chat): State<Arc<ChatState>>,
    ChatPath(user): ChatPath<String>,
) -> ChatResult<StatusCode> {
    if !users::valid_user_id(&user) {
        return Err(ChatError::NotFound);
    }
    sqlx::query_file!("queries/chat/blocks/delete.sql", caller.user_id, user)
        .execute(&chat.pg)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn blocked_between(chat: &ChatState, user: &str, other: &str) -> ChatResult<bool> {
    Ok(
        sqlx::query_file_scalar!("queries/chat/blocks/between.sql", user, other)
            .fetch_one(&chat.pg)
            .await?,
    )
}

pub async fn blocked_by(chat: &ChatState, user: &str, other: &str) -> ChatResult<bool> {
    Ok(
        sqlx::query_file_scalar!("queries/chat/blocks/exists.sql", user, other)
            .fetch_one(&chat.pg)
            .await?,
    )
}

pub async fn list(chat: &ChatState, user: &str, limit: i64) -> ChatResult<Vec<BlockedView>> {
    let rows = sqlx::query_file!("queries/chat/blocks/list.sql", user, limit)
        .fetch_all(&chat.pg)
        .await?;
    Ok(rows
        .into_iter()
        .map(|row| BlockedView {
            user_id: row.blocked_user,
            username: row.username,
            avatar_url: row.avatar_url,
            at: row.created_at.timestamp_millis(),
        })
        .collect())
}

#[cfg(test)]
#[path = "tests/blocks_tests.rs"]
mod tests;
