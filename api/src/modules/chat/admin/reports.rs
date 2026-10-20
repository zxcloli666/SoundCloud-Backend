use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use chrono::{DateTime, TimeZone, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::modules::chat::admin::audit;
use crate::modules::chat::admin::auth::ChatAdmin;
use crate::modules::chat::body::{ChatJson, ChatPath, ChatQuery};
use crate::modules::chat::moderation::actions;
use crate::modules::chat::moderation::decide::Ban;
use crate::modules::chat::moderation::evidence::Line;
use crate::modules::chat::moderation::keys::ChatKeys;
use crate::modules::chat::reports::{self, EVIDENCE_KEEP_DAYS, ReportRow};
use crate::modules::chat::{ChatError, ChatResult, ChatState, moderation, users};

pub const PAGE_MAX: i64 = 100;
pub const AUDIT_SHOWN: i64 = 200;
pub const BAN_DAYS_MAX: i64 = 3_650;

#[derive(Debug, Clone, Deserialize)]
pub struct ListQuery {
    pub state: Option<String>,
    pub before: Option<i64>,
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
    pub lines: Vec<Line>,
    pub audit: Vec<AuditView>,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AdminDecision {
    Reject,
    Uphold,
    Reopen,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Resolve {
    pub decision: AdminDecision,
    #[serde(default)]
    pub delete_seqs: Vec<i64>,
    pub ban_days: Option<i64>,
    #[serde(default)]
    pub ban_permanent: bool,
    #[serde(default)]
    pub lift_restriction: bool,
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
        .and_then(|ms| Utc.timestamp_millis_opt(ms).single())
        .unwrap_or_else(|| Utc::now() + chrono::Duration::seconds(1));
    let limit = query.limit.unwrap_or(50).clamp(1, PAGE_MAX);
    let rows = sqlx::query_file!("queries/chat/admin/reports_list.sql", state, before, limit)
        .fetch_all(&chat.pg)
        .await?;
    Ok(Json(
        rows.into_iter()
            .map(|row| ReportSummary {
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
            })
            .collect(),
    ))
}

pub async fn show(
    admin: ChatAdmin,
    State(chat): State<Arc<ChatState>>,
    ChatPath(id): ChatPath<Uuid>,
) -> ChatResult<Json<ReportDetail>> {
    Ok(Json(detail(&chat, &chat.keks, &admin.admin, id).await?))
}

pub async fn detail(
    chat: &ChatState,
    keys: &dyn ChatKeys,
    admin: &str,
    id: Uuid,
) -> ChatResult<ReportDetail> {
    let report = reports::load(chat, id).await?.ok_or(ChatError::NotFound)?;
    let lines = if report.evidence.is_some() {
        moderation::decrypt(chat, keys, &report, admin).await?
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

pub async fn resolve(
    admin: ChatAdmin,
    State(chat): State<Arc<ChatState>>,
    ChatPath(id): ChatPath<Uuid>,
    ChatJson(input): ChatJson<Resolve>,
) -> ChatResult<Json<ReportSummary>> {
    Ok(Json(apply(&chat, &admin.admin, id, &input).await?))
}

pub async fn apply(
    chat: &ChatState,
    admin: &str,
    id: Uuid,
    input: &Resolve,
) -> ChatResult<ReportSummary> {
    let report = reports::load(chat, id).await?.ok_or(ChatError::NotFound)?;
    let ban = requested_ban(input)?;
    if input
        .delete_seqs
        .iter()
        .any(|seq| !(report.seq_from..=report.seq_to).contains(seq))
    {
        return Err(ChatError::Invalid);
    }
    if input.decision != AdminDecision::Uphold
        && (!input.delete_seqs.is_empty() || ban != Ban::None)
    {
        return Err(ChatError::Invalid);
    }
    let target = audit::report(id);
    if input.decision == AdminDecision::Uphold {
        uphold(chat, admin, &report, &input.delete_seqs, ban).await?;
    }
    if input.lift_restriction {
        users::lift(chat, &report.target_user).await?;
        audit::record(
            chat,
            admin,
            "lift",
            &audit::user(&report.target_user),
            json!({"report": id}),
        )
        .await?;
    }
    let state = match input.decision {
        AdminDecision::Reject => "rejected",
        AdminDecision::Uphold => "actioned",
        AdminDecision::Reopen => "open",
    };
    sqlx::query_file_scalar!(
        "queries/chat/admin/resolve.sql",
        id,
        state,
        admin,
        EVIDENCE_KEEP_DAYS as i32
    )
    .fetch_optional(&chat.pg)
    .await?
    .ok_or(ChatError::NotFound)?;
    audit::record(
        chat,
        admin,
        state,
        &target,
        json!({"previous": report.state, "previous_by": report.handled_by}),
    )
    .await?;
    let report = reports::load(chat, id).await?.ok_or(ChatError::NotFound)?;
    Ok(summary(&report))
}

async fn uphold(
    chat: &ChatState,
    admin: &str,
    report: &ReportRow,
    seqs: &[i64],
    ban: Ban,
) -> ChatResult<()> {
    if !seqs.is_empty() {
        let removed =
            actions::remove_messages(chat, report.conversation_id, &report.target_user, seqs)
                .await?;
        audit::record(
            chat,
            admin,
            "delete_messages",
            &audit::report(report.id),
            json!({"conversation": report.conversation_id, "seqs": removed}),
        )
        .await?;
    }
    if let Some(restriction) = actions::ban(chat, &report.target_user, ban).await? {
        audit::record(
            chat,
            admin,
            "ban",
            &audit::user(&report.target_user),
            json!({
                "report": report.id,
                "permanent": ban == Ban::Permanent,
                "until": restriction.until.timestamp_millis(),
            }),
        )
        .await?;
    }
    Ok(())
}

fn requested_ban(input: &Resolve) -> ChatResult<Ban> {
    match (input.ban_permanent, input.ban_days) {
        (true, None) => Ok(Ban::Permanent),
        (false, Some(days)) if (1..=BAN_DAYS_MAX).contains(&days) => Ok(Ban::Days(days)),
        (false, None) => Ok(Ban::None),
        _ => Err(ChatError::Invalid),
    }
}

async fn audit_rows(chat: &ChatState, id: Uuid) -> ChatResult<Vec<AuditView>> {
    let rows = sqlx::query_file!(
        "queries/chat/admin/report_audits.sql",
        audit::report(id),
        AUDIT_SHOWN
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

fn summary(report: &ReportRow) -> ReportSummary {
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
    }
}
