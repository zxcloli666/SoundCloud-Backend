use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use serde::Serialize;
use uuid::Uuid;

use crate::modules::chat::{ChatCaller, ChatResult, ChatState};

pub const MESSAGES_BEFORE_ACCEPT: i64 = 3;
pub const SESSION_MIN_AGE_SECS: i32 = 3_600;
pub const REQUESTS_SHOWN: i64 = 200;
pub const REQUEST_ROW_BYTES: usize = 256;

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct RequestView {
    pub conversation_id: Uuid,
    pub kind: String,
    pub from: String,
    pub since: i64,
    pub last_seq: i64,
    pub last_at: i64,
    pub member_count: i32,
    #[serde(with = "crate::modules::chat::b64::option")]
    pub meta_ct: Option<Vec<u8>>,
    pub meta_epoch: Option<i32>,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct RequestsView {
    pub requests: Vec<RequestView>,
    pub count: usize,
}

pub async fn list(
    caller: ChatCaller,
    State(chat): State<Arc<ChatState>>,
) -> ChatResult<Json<RequestsView>> {
    let rows = sqlx::query_file!(
        "queries/chat/requests/list.sql",
        caller.user_id,
        REQUESTS_SHOWN
    )
    .fetch_all(&chat.pg)
    .await?;
    let count = rows.len();
    let mut left = caller.max_bytes as usize;
    let mut requests = Vec::with_capacity(count);
    for row in rows {
        let cost = REQUEST_ROW_BYTES + row.meta_ct.as_ref().map_or(0, |meta| meta.len() * 4 / 3);
        if cost > left {
            break;
        }
        left -= cost;
        requests.push(RequestView {
            conversation_id: row.id,
            kind: row.kind,
            from: row.from_user,
            since: row.updated_at.timestamp_millis(),
            last_seq: row.last_seq,
            last_at: row.last_at.timestamp_millis(),
            member_count: row.member_count,
            meta_ct: row.meta_ct,
            meta_epoch: row.meta_epoch,
        });
    }
    Ok(Json(RequestsView { requests, count }))
}

pub async fn session_is_young(chat: &ChatState, session: Uuid) -> ChatResult<bool> {
    let young = sqlx::query_file_scalar!(
        "queries/chat/requests/session_young.sql",
        session,
        SESSION_MIN_AGE_SECS
    )
    .fetch_optional(&chat.pg)
    .await?;
    Ok(young.unwrap_or(false))
}

#[cfg(test)]
#[path = "tests/requests_tests.rs"]
mod tests;
