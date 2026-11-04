use std::collections::BTreeSet;
use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::PgConnection;
use uuid::Uuid;

use crate::modules::chat::admin::audit;
use crate::modules::chat::admin::auth::ChatAdmin;
use crate::modules::chat::admin::evidence::{self, MediaKind};
use crate::modules::chat::admin::filed::{self, ReportKind};
use crate::modules::chat::body::{ChatJson, ChatPath};
use crate::modules::chat::media;
use crate::modules::chat::media::ledger;
use crate::modules::chat::media::limits::{ALBUM_MAX, ROOM_ALBUM_MAX};
use crate::modules::chat::moderation::evidence::SealedMessage;
use crate::modules::chat::moderation::keys::ChatKeys;
use crate::modules::chat::reports::ReportRow;
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
    let report = filed::load(chat, id).await?;
    let row = &report.row;
    let range = row.seq_from..=row.seq_to;
    if report.kind == ReportKind::Backdrop
        || seqs.is_empty()
        || seqs.iter().any(|seq| !range.contains(seq))
    {
        return Err(ChatError::Invalid);
    }
    let bundle = evidence::bundle(keys, row)?;
    let wanted: BTreeSet<i64> = seqs.iter().copied().collect();
    let carried = evidence::per_message(&bundle, report.meta_kind());
    let album_max = if report.in_room {
        ROOM_ALBUM_MAX
    } else {
        ALBUM_MAX
    };
    let mut seqs = Vec::new();
    let mut bodies = Vec::new();
    let mut images = Vec::new();
    for (message, media) in bundle.messages.iter().zip(carried) {
        if !restorable(row, &wanted, message) {
            continue;
        }
        seqs.push(message.seq);
        bodies.push(message.body.clone());
        images.extend(
            media
                .into_iter()
                .filter(|sealed| sealed.shown.kind == MediaKind::Image)
                .take(album_max)
                .map(|sealed| (sealed.shown.id, message.seq)),
        );
    }
    let mut tx = chat.pg.begin().await?;
    let restored = sqlx::query_file_scalar!(
        "queries/chat/admin/restore_messages.sql",
        row.conversation_id,
        row.target_user,
        &seqs,
        &bodies
    )
    .fetch_all(&mut *tx)
    .await?;
    let (ids, at): (Vec<Uuid>, Vec<i64>) = images
        .into_iter()
        .filter(|(_, seq)| restored.contains(seq))
        .unzip();
    let media = reattach(&mut tx, row, &ids, &at).await?;
    tx.commit().await?;
    if !restored.is_empty() {
        chat.hot.forget(row.conversation_id);
    }
    audit::record(
        chat,
        admin,
        "restore",
        &audit::report(id),
        json!({"conversation": row.conversation_id, "seqs": restored, "media": media}),
    )
    .await?;
    Ok(Restored { restored })
}

async fn reattach(
    tx: &mut PgConnection,
    report: &ReportRow,
    ids: &[Uuid],
    seqs: &[i64],
) -> ChatResult<Vec<Uuid>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    ledger::lock(tx, ids).await?;
    let reattached = sqlx::query_file_scalar!(
        "queries/chat/admin/restore_refs.sql",
        report.conversation_id,
        report.target_user,
        ids,
        seqs
    )
    .fetch_all(&mut *tx)
    .await?;
    if !reattached.is_empty() {
        media::hooks::queue_trim(&mut *tx, report.conversation_id).await?;
    }
    Ok(reattached)
}

fn restorable(report: &ReportRow, wanted: &BTreeSet<i64>, message: &SealedMessage) -> bool {
    let targeted = wanted.contains(&message.seq)
        || (message.class == "edit" && message.target_seq.is_some_and(|seq| wanted.contains(&seq)));
    message.sender == report.target_user
        && (report.seq_from..=report.seq_to).contains(&message.seq)
        && targeted
}
