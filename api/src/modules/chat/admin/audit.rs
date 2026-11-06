use serde_json::Value;
use sqlx::PgConnection;
use uuid::Uuid;

use crate::modules::chat::{ChatResult, ChatState};

pub async fn record(
    chat: &ChatState,
    admin: &str,
    action: &str,
    target: &str,
    detail: Value,
) -> ChatResult<()> {
    let mut conn = chat.pg.acquire().await?;
    record_in(&mut conn, admin, action, target, detail).await
}

pub async fn record_in(
    conn: &mut PgConnection,
    admin: &str,
    action: &str,
    target: &str,
    detail: Value,
) -> ChatResult<()> {
    sqlx::query_file!(
        "queries/chat/admin/audit_insert.sql",
        Uuid::now_v7(),
        admin,
        action,
        target,
        detail
    )
    .execute(&mut *conn)
    .await?;
    Ok(())
}

pub fn report(id: Uuid) -> String {
    format!("report:{id}")
}

pub fn user(id: &str) -> String {
    format!("user:{id}")
}

pub fn message(conversation: Uuid, seq: i64) -> String {
    format!("message:{conversation}:{seq}")
}

pub fn room(code: &str) -> String {
    format!("room:{code}")
}
