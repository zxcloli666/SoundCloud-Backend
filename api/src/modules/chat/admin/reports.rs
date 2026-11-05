use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use chrono::{DateTime, TimeZone, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::modules::chat::admin::audit;
use crate::modules::chat::admin::auth::ChatAdmin;
use crate::modules::chat::admin::evidence::{self, EvidenceLine};
use crate::modules::chat::admin::filed::{self, Filed};
use crate::modules::chat::body::{ChatPath, ChatQuery};
use crate::modules::chat::moderation::keys::ChatKeys;
use crate::modules::chat::reports::ReportKind;
use crate::modules::chat::{ChatError, ChatResult, ChatState, moderation};

pub const PAGE_MAX: i64 = 100;
pub const AUDIT_SHOWN: i64 = 200;
pub const VIEWS_SHOWN: i64 = 50;

#[derive(Debug, Clone, Deserialize)]
pub struct ListQuery {
    pub state: Option<String>,
    pub before: Option<i64>,
    pub before_id: Option<Uuid>,
    pub limit: Option<i64>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ReportSummary {
    pub id: Uuid,
    pub reporter: String,
    pub target_user: String,
    pub conversation_id: Uuid,
    pub seq_from: i64,
    pub seq_to: i64,
    pub reason: String,
    pub comment: Option<String>,
    pub weight: f32,
    pub state: String,
    pub created_at: i64,
    pub handled_by: Option<String>,
    pub handled_at: Option<i64>,
    pub has_evidence: bool,
    pub kind: ReportKind,
    pub media_id: Option<Uuid>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AuditView {
    pub admin: String,
    pub action: String,
    pub detail: Value,
    pub at: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct ReportDetail {
    pub report: ReportSummary,
    pub lines: Vec<EvidenceLine>,
    pub audit: Vec<AuditView>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ReportTrail {
    pub report: ReportSummary,
    pub audit: Vec<AuditView>,
}

pub async fn list(
    _: ChatAdmin,
    State(chat): State<Arc<ChatState>>,
    ChatQuery(query): ChatQuery<ListQuery>,
) -> ChatResult<Json<Vec<ReportSummary>>> {
    let state = query.state.unwrap_or_else(|| "open".to_owned());
    if !matches!(state.as_str(), "open" | "actioned" | "rejected" | "all") {
        return Err(ChatError::Invalid);
    }
    let before = query
        .before
        .filter(|_| query.before_id.is_none())
        .and_then(|ms| Utc.timestamp_millis_opt(ms).single())
        .unwrap_or_else(|| Utc::now() + chrono::Duration::seconds(1));
    let limit = query.limit.unwrap_or(50).clamp(1, PAGE_MAX);
    let rows = sqlx::query_file!(
        "queries/chat/admin/reports_list.sql",
        state,
        before,
        limit,
        query.before_id
    )
    .fetch_all(&chat.pg)
    .await?;
    rows.into_iter()
        .map(|row| {
            Ok(ReportSummary {
                id: row.id,
                reporter: row.reporter,
                target_user: row.target_user,
                conversation_id: row.conversation_id,
                seq_from: row.seq_from,
                seq_to: row.seq_to,
                reason: row.reason,
                comment: row.comment,
                weight: row.weight,
                state: row.state,
                created_at: row.created_at.timestamp_millis(),
                handled_by: row.handled_by,
                handled_at: row.handled_at.map(|at| at.timestamp_millis()),
                has_evidence: row.has_evidence,
                kind: ReportKind::parse(&row.kind)?,
                media_id: row.media_id,
            })
        })
        .collect::<ChatResult<Vec<ReportSummary>>>()
        .map(Json)
}

pub async fn show(
    admin: ChatAdmin,
    State(chat): State<Arc<ChatState>>,
    ChatPath(id): ChatPath<Uuid>,
) -> ChatResult<Json<ReportDetail>> {
    Ok(Json(detail(&chat, &chat.keks, &admin.admin, id).await?))
}

pub async fn trail(
    _: ChatAdmin,
    State(chat): State<Arc<ChatState>>,
    ChatPath(id): ChatPath<Uuid>,
) -> ChatResult<Json<ReportTrail>> {
    let report = filed::load(&chat, id).await?;
    Ok(Json(ReportTrail {
        report: summary(&report),
        audit: audit_rows(&chat, id).await?,
    }))
}

pub async fn detail(
    chat: &ChatState,
    keys: &dyn ChatKeys,
    admin: &str,
    id: Uuid,
) -> ChatResult<ReportDetail> {
    let report = filed::load(chat, id).await?;
    let lines = if report.row.evidence.is_some() {
        let lines = moderation::decrypt(chat, keys, &report.row, admin).await?;
        let bundle = evidence::bundle(keys, &report.row)?;
        let mut shown = evidence::shown_lines(lines, &bundle, report.meta_kind());
        evidence::label_stored(&chat.pg, &mut shown).await?;
        shown
    } else {
        Vec::new()
    };
    let audit = audit_rows(chat, id).await?;
    Ok(ReportDetail {
        report: summary(&report),
        lines,
        audit,
    })
}

pub async fn audit_rows(chat: &ChatState, id: Uuid) -> ChatResult<Vec<AuditView>> {
    let rows = sqlx::query_file!(
        "queries/chat/admin/report_audits.sql",
        audit::report(id),
        id.to_string(),
        AUDIT_SHOWN,
        VIEWS_SHOWN
    )
    .fetch_all(&chat.pg)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| AuditView {
            admin: row.admin,
            action: row.action,
            detail: row.detail,
            at: row.created_at.timestamp_millis(),
        })
        .collect())
}

pub fn summary(filed: &Filed) -> ReportSummary {
    let report = &filed.row;
    ReportSummary {
        id: report.id,
        reporter: report.reporter.clone(),
        target_user: report.target_user.clone(),
        conversation_id: report.conversation_id,
        seq_from: report.seq_from,
        seq_to: report.seq_to,
        reason: report.reason.clone(),
        comment: report.comment.clone(),
        weight: report.weight,
        state: report.state.clone(),
        created_at: report.created_at.timestamp_millis(),
        handled_by: report.handled_by.clone(),
        handled_at: report
            .handled_at
            .map(|at: DateTime<Utc>| at.timestamp_millis()),
        has_evidence: report.evidence.is_some(),
        kind: filed.kind,
        media_id: filed.media_id,
    }
}
