use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::PgConnection;
use uuid::Uuid;

use crate::modules::chat::admin::audit;
use crate::modules::chat::body::ChatQuery;
use crate::modules::chat::limits::{self, Bucket};
use crate::modules::chat::model::MemberRole;
use crate::modules::chat::public_chat::{self, staff};
use crate::modules::chat::{ChatCaller, ChatResult, ChatState};

pub const PAGE_DEFAULT: u32 = 50;
pub const PAGE_MAX: u32 = 200;
const PREFIX: &str = "public.";
const ACTIONS: &str = "chat_public_moderation_total";

#[derive(Debug, Clone, Deserialize)]
pub struct LogQuery {
    pub before: Option<Uuid>,
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct LogEntry {
    pub id: Uuid,
    pub by: String,
    pub action: String,
    pub target: String,
    pub detail: Value,
    pub ts: i64,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct LogPage {
    pub entries: Vec<LogEntry>,
    pub next: Option<Uuid>,
}

pub async fn record_in(
    conn: &mut PgConnection,
    by: &str,
    action: &str,
    target: &str,
    detail: Value,
) -> ChatResult<()> {
    audit::record_in(conn, by, &format!("{PREFIX}{action}"), target, detail).await?;
    metrics::counter!(ACTIONS, "action" => action.to_owned()).increment(1);
    Ok(())
}

pub async fn record(
    chat: &ChatState,
    by: &str,
    action: &str,
    target: &str,
    detail: Value,
) -> ChatResult<()> {
    let mut conn = chat.pg.acquire().await?;
    record_in(&mut conn, by, action, target, detail).await
}

pub async fn read(
    caller: ChatCaller,
    State(chat): State<Arc<ChatState>>,
    ChatQuery(query): ChatQuery<LogQuery>,
) -> ChatResult<Json<LogPage>> {
    public_chat::ensure_on(&chat)?;
    limits::admit(&chat, Bucket::ReadList, &caller.user_id).await?;
    let role = staff::staff_role(&chat, &caller.user_id).await?;
    let own = (role != MemberRole::Owner).then_some(caller.user_id.as_str());
    let limit = query.limit.unwrap_or(PAGE_DEFAULT).clamp(1, PAGE_MAX);
    let mut rows = sqlx::query_file!(
        "queries/chat/public/log.sql",
        own,
        query.before,
        i64::from(limit) + 1
    )
    .fetch_all(&chat.pg)
    .await?;
    let more = rows.len() > limit as usize;
    rows.truncate(limit as usize);
    let next = rows.last().map(|row| row.id).filter(|_| more);
    let entries = rows
        .into_iter()
        .map(|row| LogEntry {
            id: row.id,
            by: row.admin,
            action: row
                .action
                .strip_prefix(PREFIX)
                .unwrap_or(&row.action)
                .to_owned(),
            target: row.target,
            detail: row.detail,
            ts: row.created_at.timestamp_millis(),
        })
        .collect();
    Ok(Json(LogPage { entries, next }))
}
