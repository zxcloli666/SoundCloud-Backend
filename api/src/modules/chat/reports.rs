use std::collections::BTreeSet;
use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::modules::chat::body::ChatJson;
use crate::modules::chat::limits::{self, Bucket};
use crate::modules::chat::moderation::evidence::{self, StoredMessage};
use crate::modules::chat::moderation::keys::ChatKeys;
use crate::modules::chat::{ChatCaller, ChatError, ChatResult, ChatState, rooms, users};

pub const CONTEXT_MESSAGES: i64 = 10;
pub const EVIDENCE_KEEP_DAYS: i64 = 180;
pub const RANGE_MAX: i64 = 50;
pub const COMMENT_CHARS_MAX: usize = 500;
pub const REASONS: [&str; 5] = ["spam", "harassment", "doxxing", "illegal", "other"];
pub const SETTLED_DAYS: i64 = 7;
pub const NEW_REPORTER_WEIGHT: f32 = 0.25;
pub const FALSE_REPORTS_TO_SILENCE: i64 = 2;
pub const SILENCE_DAYS: i32 = 30;

#[derive(Debug, Clone, Deserialize)]
pub struct FileReport {
    pub conversation_id: Uuid,
    pub seq_from: i64,
    pub seq_to: i64,
    pub reason: String,
    pub comment: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct FiledReport {
    pub id: Uuid,
    pub state: &'static str,
    pub duplicate: bool,
}

#[derive(Clone)]
pub struct ReportRow {
    pub id: Uuid,
    pub reporter: String,
    pub target_user: String,
    pub conversation_id: Uuid,
    pub seq_from: i64,
    pub seq_to: i64,
    pub reason: String,
    pub comment: Option<String>,
    pub weight: f32,
    pub evidence: Option<Vec<u8>>,
    pub evidence_kek_version: Option<i16>,
    pub state: String,
    pub created_at: DateTime<Utc>,
    pub handled_by: Option<String>,
    pub handled_at: Option<DateTime<Utc>>,
}

pub async fn file(
    caller: ChatCaller,
    State(chat): State<Arc<ChatState>>,
    ChatJson(input): ChatJson<FileReport>,
) -> ChatResult<Json<FiledReport>> {
    validate(&input)?;
    limits::admit(&chat, Bucket::Report, &caller.user_id).await?;
    let filed = file_with(&chat, &chat.keks, &caller.user_id, &input).await?;
    if !filed.duplicate {
        chat.moderator.nudge();
    }
    Ok(Json(filed))
}

pub fn validate(input: &FileReport) -> ChatResult<()> {
    let span = input.seq_to - input.seq_from;
    let comment_fits = input
        .comment
        .as_ref()
        .is_none_or(|comment| comment.chars().count() <= COMMENT_CHARS_MAX);
    if input.seq_from < 1
        || !(0..RANGE_MAX).contains(&span)
        || !REASONS.contains(&input.reason.as_str())
        || !comment_fits
    {
        return Err(ChatError::Invalid);
    }
    Ok(())
}

pub async fn file_with(
    chat: &ChatState,
    keys: &dyn ChatKeys,
    reporter: &str,
    input: &FileReport,
) -> ChatResult<FiledReport> {
    let floor = visible_from(chat, reporter, input.conversation_id).await?;
    if input.seq_from <= floor {
        return Err(ChatError::Invalid);
    }
    let reported = range(chat, input).await?;
    let target = single_target(&reported, reporter).ok_or(ChatError::Invalid)?;

    if let Some(id) = sqlx::query_file_scalar!(
        "queries/chat/reports/duplicate.sql",
        reporter,
        input.conversation_id,
        target,
        input.seq_from,
        input.seq_to
    )
    .fetch_optional(&chat.pg)
    .await?
    {
        return Ok(FiledReport {
            id,
            state: "open",
            duplicate: true,
        });
    }

    let mut messages = context(chat, input, floor).await?;
    messages.extend(reported);
    let capture = evidence::capture(
        chat,
        keys,
        input.conversation_id,
        (input.seq_from, input.seq_to),
        &messages,
    )
    .await?;
    users::touch(chat, reporter).await?;
    let weight = reporter_weight(chat, reporter).await?;
    let id = Uuid::now_v7();
    sqlx::query_file!(
        "queries/chat/reports/insert.sql",
        id,
        reporter,
        target,
        input.conversation_id,
        input.seq_from,
        input.seq_to,
        input.reason,
        input.comment.as_deref().map(str::trim).unwrap_or_default(),
        weight,
        capture.sealed,
        i16::try_from(capture.version).map_err(|_| ChatError::Internal)?
    )
    .execute(&chat.pg)
    .await?;
    metrics::counter!("chat_reports_total", "reason" => reason_label(&input.reason)).increment(1);
    Ok(FiledReport {
        id,
        state: "open",
        duplicate: false,
    })
}

async fn visible_from(chat: &ChatState, reporter: &str, conversation: Uuid) -> ChatResult<i64> {
    let found = sqlx::query_file!("queries/chat/reports/conversation.sql", conversation)
        .fetch_optional(&chat.pg)
        .await?
        .filter(|row| row.live)
        .ok_or(ChatError::NotFound)?;
    match found.kind.as_str() {
        "direct" | "group" => {
            let member = sqlx::query_file!(
                "queries/chat/reports/membership.sql",
                conversation,
                reporter
            )
            .fetch_optional(&chat.pg)
            .await?
            .filter(|member| matches!(member.state.as_str(), "active" | "invited"))
            .ok_or(ChatError::NotMember)?;
            Ok(member.joined_seq)
        }
        "room" => rooms::member_from_seq(chat, conversation, reporter)
            .await?
            .ok_or(ChatError::NotMember),
        _ => Err(ChatError::Invalid),
    }
}

async fn range(chat: &ChatState, input: &FileReport) -> ChatResult<Vec<StoredMessage>> {
    Ok(sqlx::query_file_as!(
        StoredMessage,
        "queries/chat/reports/range.sql",
        input.conversation_id,
        input.seq_from,
        input.seq_to
    )
    .fetch_all(&chat.pg)
    .await?)
}

async fn context(
    chat: &ChatState,
    input: &FileReport,
    floor: i64,
) -> ChatResult<Vec<StoredMessage>> {
    let mut rows = sqlx::query_file_as!(
        StoredMessage,
        "queries/chat/reports/context.sql",
        input.conversation_id,
        input.seq_from,
        floor,
        CONTEXT_MESSAGES
    )
    .fetch_all(&chat.pg)
    .await?;
    rows.reverse();
    Ok(rows)
}

fn single_target(reported: &[StoredMessage], reporter: &str) -> Option<String> {
    let senders: BTreeSet<&str> = reported
        .iter()
        .filter(|message| message.class != "system" && message.sender_user != reporter)
        .map(|message| message.sender_user.as_str())
        .collect();
    let mut senders = senders.into_iter();
    match (senders.next(), senders.next()) {
        (Some(target), None) => Some(target.to_owned()),
        _ => None,
    }
}

pub async fn reporter_weight(chat: &ChatState, reporter: &str) -> ChatResult<f32> {
    let rejected = sqlx::query_file_scalar!(
        "queries/chat/reports/rejected_by.sql",
        reporter,
        SILENCE_DAYS
    )
    .fetch_one(&chat.pg)
    .await?;
    if rejected >= FALSE_REPORTS_TO_SILENCE {
        return Ok(0.0);
    }
    let settled = users::first_seen(chat, reporter)
        .await?
        .is_some_and(|seen| seen <= Utc::now() - Duration::days(SETTLED_DAYS));
    Ok(if settled { 1.0 } else { NEW_REPORTER_WEIGHT })
}

pub async fn load(chat: &ChatState, id: Uuid) -> ChatResult<Option<ReportRow>> {
    Ok(
        sqlx::query_file_as!(ReportRow, "queries/chat/reports/load.sql", id)
            .fetch_optional(&chat.pg)
            .await?,
    )
}

fn reason_label(reason: &str) -> &'static str {
    REASONS
        .iter()
        .find(|known| **known == reason)
        .copied()
        .unwrap_or("other")
}

#[cfg(test)]
#[path = "tests/reports_tests.rs"]
mod tests;
