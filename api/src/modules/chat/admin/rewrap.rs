use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use serde::Serialize;
use serde_json::json;
use uuid::Uuid;

use crate::modules::chat::admin::audit;
use crate::modules::chat::admin::auth::ChatAdmin;
use crate::modules::chat::moderation::keys::ChatKeys;
use crate::modules::chat::{ChatError, ChatResult, ChatState};

pub const KEYS_PER_CALL: i64 = 1_000;
pub const EVIDENCE_PER_CALL: i64 = 100;
pub const AUDIT_TARGET: &str = "keys";

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Rewrapped {
    pub keys: u64,
    pub evidence: u64,
}

pub async fn rewrap(
    admin: ChatAdmin,
    State(chat): State<Arc<ChatState>>,
) -> ChatResult<Json<Rewrapped>> {
    Ok(Json(rewrap_with(&chat, &chat.keks, &admin.admin).await?))
}

pub async fn rewrap_with(
    chat: &ChatState,
    keys: &dyn ChatKeys,
    admin: &str,
) -> ChatResult<Rewrapped> {
    let done = Rewrapped {
        keys: conversation_keys(chat, keys).await?,
        evidence: evidence(chat, keys).await?,
    };
    if done.keys > 0 || done.evidence > 0 {
        audit::record(
            chat,
            admin,
            "rewrap",
            AUDIT_TARGET,
            json!({
                "keys": done.keys,
                "evidence": done.evidence,
                "kek": keys.kek_current(),
                "reports_kek": keys.reports_current(),
            }),
        )
        .await?;
    }
    Ok(done)
}

async fn conversation_keys(chat: &ChatState, keys: &dyn ChatKeys) -> ChatResult<u64> {
    let current = version(keys.kek_current())?;
    let rows = sqlx::query_file!(
        "queries/chat/admin/rewrap_keys_batch.sql",
        current,
        KEYS_PER_CALL
    )
    .fetch_all(&chat.pg)
    .await?;
    let mut conversations: Vec<Uuid> = Vec::with_capacity(rows.len());
    let mut epochs: Vec<i32> = Vec::with_capacity(rows.len());
    let mut wrapped: Vec<Vec<u8>> = Vec::with_capacity(rows.len());
    for row in rows {
        let old = u16::try_from(row.kek_version).map_err(|_| ChatError::Internal)?;
        let ck = keys.unwrap_ck(old, row.conversation_id, row.epoch, &row.wrapped)?;
        wrapped.push(keys.wrap_ck(row.conversation_id, row.epoch, &ck)?);
        conversations.push(row.conversation_id);
        epochs.push(row.epoch);
    }
    if conversations.is_empty() {
        return Ok(0);
    }
    let updated = sqlx::query_file!(
        "queries/chat/admin/rewrap_keys_update.sql",
        &conversations,
        &epochs,
        &wrapped,
        current
    )
    .execute(&chat.pg)
    .await?;
    Ok(updated.rows_affected())
}

async fn evidence(chat: &ChatState, keys: &dyn ChatKeys) -> ChatResult<u64> {
    let current = version(keys.reports_current())?;
    let rows = sqlx::query_file!(
        "queries/chat/admin/rewrap_evidence_batch.sql",
        current,
        EVIDENCE_PER_CALL
    )
    .fetch_all(&chat.pg)
    .await?;
    let mut done = 0;
    for row in rows {
        let old = u16::try_from(row.evidence_kek_version).map_err(|_| ChatError::Internal)?;
        let plain = keys.open_evidence(old, &row.evidence)?;
        let sealed = keys.seal_evidence(&plain)?;
        drop(plain);
        done += sqlx::query_file!(
            "queries/chat/admin/rewrap_evidence_update.sql",
            row.id,
            sealed,
            current,
            row.evidence_kek_version
        )
        .execute(&chat.pg)
        .await?
        .rows_affected();
    }
    Ok(done)
}

fn version(raw: u16) -> ChatResult<i16> {
    i16::try_from(raw).map_err(|_| ChatError::Internal)
}
