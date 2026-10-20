use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt;
use serde_json::json;
use uuid::Uuid;

use crate::modules::chat::admin::audit;
use crate::modules::chat::moderation::actions::{self, Closing};
use crate::modules::chat::moderation::decide::{self, Ban, Decision, Facts};
use crate::modules::chat::moderation::keys::ChatKeys;
use crate::modules::chat::moderation::{AUDIT_ADMIN, CALLS_IN_FLIGHT, history, rules};
use crate::modules::chat::reports::{self, ReportRow};
use crate::modules::chat::sweeps::leader;
use crate::modules::chat::{ChatError, ChatResult, ChatState};

pub const BATCH: i64 = 16;
pub const IDLE: Duration = Duration::from_secs(5);

pub async fn run(chat: Arc<ChatState>) {
    loop {
        if matches!(leader::hold(&chat).await, Ok(true)) {
            match run_once(&chat, &chat.keks).await {
                Ok(0) => {}
                Ok(done) => tracing::info!(done, "chat reports moderated"),
                Err(error) => tracing::warn!(code = error.code(), "chat moderation pass failed"),
            }
        }
        tokio::select! {
            _ = chat.shutdown.cancelled() => return,
            _ = tokio::time::sleep(IDLE) => {}
            _ = chat.moderator.woken() => {}
        }
    }
}

pub async fn run_once(chat: &ChatState, keys: &dyn ChatKeys) -> ChatResult<usize> {
    let pending = sqlx::query_file_scalar!("queries/chat/reports/pending.sql", BATCH)
        .fetch_all(&chat.pg)
        .await?;
    let results: Vec<ChatResult<Decision>> = futures::stream::iter(pending)
        .map(|id| moderate(chat, keys, id))
        .buffer_unordered(CALLS_IN_FLIGHT)
        .collect()
        .await;
    let mut done = 0;
    for result in results {
        match result {
            Ok(_) => done += 1,
            Err(error) => tracing::warn!(code = error.code(), "a chat report was not moderated"),
        }
    }
    Ok(done)
}

pub async fn moderate(chat: &ChatState, keys: &dyn ChatKeys, id: Uuid) -> ChatResult<Decision> {
    let report = reports::load(chat, id).await?.ok_or(ChatError::NotFound)?;
    if report.state != "open" || report.handled_by.is_some() {
        return Ok(Decision::Fallback("handled"));
    }
    let decision = if report.weight <= 0.0 {
        Decision::Fallback("silenced_reporter")
    } else if !chat.moderator.ai_enabled() {
        Decision::Fallback("rules_only")
    } else {
        match chat.moderator.examine(chat, keys, &report).await {
            Ok(judgement) => {
                let facts = Facts {
                    target_seqs: judgement.target_seqs,
                    injection: judgement.injection,
                    repeat: history::repeat_offender(chat, &report.target_user, report.id).await?,
                };
                decide::decide(judgement.answer.verdict.as_ref(), &facts)
            }
            Err(error) => {
                tracing::warn!(code = error.code(), "a chat report could not be examined");
                Decision::Fallback("unexamined")
            }
        }
    };
    apply(chat, &report, &decision).await?;
    metrics::counter!("chat_moderation_total", "decision" => decision.label()).increment(1);
    Ok(decision)
}

pub async fn apply(chat: &ChatState, report: &ReportRow, decision: &Decision) -> ChatResult<()> {
    let target = audit::report(report.id);
    match decision {
        Decision::Reject => {
            if actions::close_open(chat, report.id, Closing::Rejected, AUDIT_ADMIN).await? {
                audit::record(chat, AUDIT_ADMIN, "reject", &target, json!({})).await?;
            }
        }
        Decision::Remove { seqs, ban } => {
            let removed =
                actions::remove_messages(chat, report.conversation_id, &report.target_user, seqs)
                    .await?;
            audit::record(
                chat,
                AUDIT_ADMIN,
                "delete_messages",
                &target,
                json!({"conversation": report.conversation_id, "seqs": removed}),
            )
            .await?;
            if let Some(restriction) = actions::ban(chat, &report.target_user, *ban).await? {
                audit::record(
                    chat,
                    AUDIT_ADMIN,
                    "ban",
                    &audit::user(&report.target_user),
                    json!({
                        "report": report.id,
                        "permanent": *ban == Ban::Permanent,
                        "until": restriction.until.timestamp_millis(),
                    }),
                )
                .await?;
            }
            if actions::close_open(chat, report.id, Closing::Actioned, AUDIT_ADMIN).await? {
                audit::record(chat, AUDIT_ADMIN, "uphold", &target, json!({})).await?;
            }
        }
        Decision::Fallback(reason) => {
            rules::apply(chat, &report.target_user, report.id).await?;
            actions::mark_judged(chat, report.id, AUDIT_ADMIN).await?;
            audit::record(
                chat,
                AUDIT_ADMIN,
                "queue",
                &target,
                json!({"reason": reason}),
            )
            .await?;
        }
    }
    Ok(())
}
