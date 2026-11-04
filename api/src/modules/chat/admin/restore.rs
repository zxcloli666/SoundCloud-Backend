use std::collections::BTreeSet;
use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use serde::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;

use crate::modules::chat::admin::audit;
use crate::modules::chat::admin::auth::ChatAdmin;
use crate::modules::chat::body::{ChatJson, ChatPath};
use crate::modules::chat::moderation::evidence::{self, SealedMessage};
use crate::modules::chat::moderation::keys::ChatKeys;
use crate::modules::chat::reports::{self, ReportRow};
use crate::modules::chat::{ChatError, ChatResult, ChatState};

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Restore {
    pub seqs: Vec<i64>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Restored {
    pub restored: Vec<i64>,
}

pub async fn restore(
    admin: ChatAdmin,
    State(chat): State<Arc<ChatState>>,
    ChatPath(id): ChatPath<Uuid>,
    ChatJson(input): ChatJson<Restore>,
) -> ChatResult<Json<Restored>> {
    Ok(Json(
        restore_with(&chat, &chat.keks, &admin.admin, id, &input.seqs).await?,
    ))
}

pub async fn restore_with(
    chat: &ChatState,
    keys: &dyn ChatKeys,
    admin: &str,
    id: Uuid,
    seqs: &[i64],
) -> ChatResult<Restored> {
    let report = reports::load(chat, id).await?.ok_or(ChatError::NotFound)?;
    let range = report.seq_from..=report.seq_to;
    if seqs.is_empty() || seqs.iter().any(|seq| !range.contains(seq)) {
        return Err(ChatError::Invalid);
    }
    let (Some(sealed), Some(version)) = (&report.evidence, report.evidence_kek_version) else {
        return Err(ChatError::NotFound);
    };
    let version = u16::try_from(version).map_err(|_| ChatError::Internal)?;
    let bundle = evidence::open(keys, version, sealed)?;
    let wanted: BTreeSet<i64> = seqs.iter().copied().collect();
    let (seqs, bodies): (Vec<i64>, Vec<Vec<u8>>) = bundle
        .messages
        .into_iter()
        .filter(|message| restorable(&report, &wanted, message))
        .map(|message| (message.seq, message.body))
        .unzip();
    let restored = sqlx::query_file_scalar!(
        "queries/chat/admin/restore_messages.sql",
        report.conversation_id,
        report.target_user,
        &seqs,
        &bodies
    )
    .fetch_all(&chat.pg)
    .await?;
    if !restored.is_empty() {
        chat.hot.forget(report.conversation_id);
    }
    audit::record(
        chat,
        admin,
        "restore",
        &audit::report(id),
        json!({"conversation": report.conversation_id, "seqs": restored}),
    )
    .await?;
    Ok(Restored { restored })
}

fn restorable(report: &ReportRow, wanted: &BTreeSet<i64>, message: &SealedMessage) -> bool {
    let targeted = wanted.contains(&message.seq)
        || (message.class == "edit" && message.target_seq.is_some_and(|seq| wanted.contains(&seq)));
    message.sender == report.target_user
        && (report.seq_from..=report.seq_to).contains(&message.seq)
        && targeted
}
