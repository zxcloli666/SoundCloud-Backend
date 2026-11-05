use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use serde::Deserialize;
use serde_json::json;
use sqlx::PgConnection;
use uuid::Uuid;

use crate::modules::chat::admin::auth::ChatAdmin;
use crate::modules::chat::admin::filed::{self, Filed};
use crate::modules::chat::admin::reports::{ReportSummary, summary};
use crate::modules::chat::admin::{audit, evidence};
use crate::modules::chat::body::{ChatJson, ChatPath};
use crate::modules::chat::media;
use crate::modules::chat::moderation::actions;
use crate::modules::chat::moderation::decide::Ban;
use crate::modules::chat::moderation::keys::ChatKeys;
use crate::modules::chat::reports::{EVIDENCE_KEEP_DAYS, ReportKind};
use crate::modules::chat::{ChatError, ChatResult, ChatState, users};

pub const BAN_DAYS_MAX: i64 = 3_650;

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
    #[serde(default)]
    pub clear_backdrop: bool,
}

pub async fn resolve(
    admin: ChatAdmin,
    State(chat): State<Arc<ChatState>>,
    ChatPath(id): ChatPath<Uuid>,
    ChatJson(input): ChatJson<Resolve>,
) -> ChatResult<Json<ReportSummary>> {
    Ok(Json(
        apply(&chat, &chat.keks, &admin.admin, id, &input).await?,
    ))
}

pub async fn apply(
    chat: &ChatState,
    keys: &dyn ChatKeys,
    admin: &str,
    id: Uuid,
    input: &Resolve,
) -> ChatResult<ReportSummary> {
    let report = filed::load(chat, id).await?;
    let ban = requested_ban(input)?;
    validate(&report, input, ban)?;
    if input.decision == AdminDecision::Uphold {
        uphold(chat, admin, &report, input, ban).await?;
    }
    let target_user = &report.row.target_user;
    if input.lift_restriction {
        users::lift(chat, target_user).await?;
        audit::record(
            chat,
            admin,
            "lift",
            &audit::user(target_user),
            json!({"report": id}),
        )
        .await?;
    }
    let state = match input.decision {
        AdminDecision::Reject => "rejected",
        AdminDecision::Uphold => "actioned",
        AdminDecision::Reopen => "open",
    };
    let shown = match (input.decision, &report.row.evidence) {
        (AdminDecision::Reopen, Some(_)) => {
            evidence::reported_media(&evidence::bundle(keys, &report.row)?)
        }
        _ => Vec::new(),
    };
    let mut tx = chat.pg.begin().await?;
    sqlx::query_file_scalar!(
        "queries/chat/admin/resolve.sql",
        id,
        state,
        admin,
        EVIDENCE_KEEP_DAYS as i32
    )
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(ChatError::NotFound)?;
    if input.decision == AdminDecision::Reopen {
        hold_again(&mut tx, &report, &shown).await?;
    } else {
        media::hooks::release_holds(&mut tx, id).await?;
    }
    tx.commit().await?;
    audit::record(
        chat,
        admin,
        state,
        &audit::report(id),
        json!({"previous": report.row.state, "previous_by": report.row.handled_by}),
    )
    .await?;
    Ok(summary(&filed::load(chat, id).await?))
}

async fn hold_again(tx: &mut PgConnection, report: &Filed, shown: &[Uuid]) -> ChatResult<()> {
    let row = &report.row;
    match report.backdrop() {
        Some(media) => media::hooks::hold(tx, row.id, &[media]).await?,
        None => {
            media::hooks::hold_for_report(
                tx,
                row.id,
                row.conversation_id,
                row.seq_from,
                row.seq_to,
                shown,
            )
            .await?
        }
    };
    Ok(())
}

fn validate(report: &Filed, input: &Resolve, ban: Ban) -> ChatResult<()> {
    let range = report.row.seq_from..=report.row.seq_to;
    let outside = input.delete_seqs.iter().any(|seq| !range.contains(seq));
    let backdrop = report.kind == ReportKind::Backdrop;
    let acts = !input.delete_seqs.is_empty() || ban != Ban::None || input.clear_backdrop;
    let refused = outside
        || (input.decision != AdminDecision::Uphold && acts)
        || (input.lift_restriction && ban != Ban::None)
        || (input.clear_backdrop && !backdrop)
        || (backdrop && !input.delete_seqs.is_empty());
    if refused {
        return Err(ChatError::Invalid);
    }
    Ok(())
}

async fn uphold(
    chat: &ChatState,
    admin: &str,
    report: &Filed,
    input: &Resolve,
    ban: Ban,
) -> ChatResult<()> {
    let row = &report.row;
    if input.clear_backdrop {
        clear_backdrop(chat, admin, report).await?;
    }
    if !input.delete_seqs.is_empty() {
        let removed = actions::remove_messages(
            chat,
            row.conversation_id,
            &row.target_user,
            &input.delete_seqs,
        )
        .await?;
        audit::record(
            chat,
            admin,
            "delete_messages",
            &audit::report(row.id),
            json!({"conversation": row.conversation_id, "seqs": removed}),
        )
        .await?;
        actions::announce_removed(chat, row.conversation_id, &removed).await;
    }
    if let Some(restriction) = actions::ban(chat, &row.target_user, ban).await? {
        audit::record(
            chat,
            admin,
            "ban",
            &audit::user(&row.target_user),
            json!({
                "report": row.id,
                "permanent": ban == Ban::Permanent,
                "until": restriction.until.timestamp_millis(),
            }),
        )
        .await?;
    }
    Ok(())
}

async fn clear_backdrop(chat: &ChatState, admin: &str, report: &Filed) -> ChatResult<()> {
    let media = report.backdrop().ok_or(ChatError::Internal)?;
    let conversation = report.row.conversation_id;
    let cleared = media::backdrop::clear(chat, conversation, media, None).await?;
    audit::record(
        chat,
        admin,
        "clear_backdrop",
        &audit::report(report.row.id),
        json!({"conversation": conversation, "media": media, "cleared": cleared}),
    )
    .await
}

fn requested_ban(input: &Resolve) -> ChatResult<Ban> {
    match (input.ban_permanent, input.ban_days) {
        (true, None) => Ok(Ban::Permanent),
        (false, Some(days)) if (1..=BAN_DAYS_MAX).contains(&days) => Ok(Ban::Days(days)),
        (false, None) => Ok(Ban::None),
        _ => Err(ChatError::Invalid),
    }
}
