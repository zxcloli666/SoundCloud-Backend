use chrono::Utc;
use serde::Deserialize;
use sqlx::PgExecutor;
use uuid::Uuid;

use super::{COMMENT_CHARS_MAX, FileReport, FiledReport, REASONS, reason_label, reporter_weight};
use crate::modules::chat::media::backdrop;
use crate::modules::chat::moderation::evidence::{self, StoredMessage};
use crate::modules::chat::moderation::keys::ChatKeys;
use crate::modules::chat::rooms::lookup::room_of_chat;
use crate::modules::chat::{ChatError, ChatResult, ChatState, users};
use crate::modules::rooms::model::{IMAGE_COOLDOWN_MS, Room};

pub const HOLD_DAYS: i32 = 30;
pub const CLEAR_REPORTERS: i64 = 3;
pub const CLEAR_WINDOW_SECS: i32 = 10 * 60;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ReportKind {
    #[default]
    Messages,
    Backdrop,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ReportBody {
    pub conversation_id: Uuid,
    #[serde(default)]
    pub kind: ReportKind,
    pub seq_from: Option<i64>,
    pub seq_to: Option<i64>,
    pub media_id: Option<Uuid>,
    pub reason: String,
    pub comment: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BackdropReport {
    pub conversation_id: Uuid,
    pub media_id: Uuid,
    pub reason: String,
    pub comment: Option<String>,
}

pub enum Filing {
    Messages(FileReport),
    Backdrop(BackdropReport),
}

impl ReportBody {
    pub fn filing(self) -> ChatResult<Filing> {
        match (self.kind, self.seq_from, self.seq_to, self.media_id) {
            (ReportKind::Messages, Some(seq_from), Some(seq_to), None) => {
                Ok(Filing::Messages(FileReport {
                    conversation_id: self.conversation_id,
                    seq_from,
                    seq_to,
                    reason: self.reason,
                    comment: self.comment,
                }))
            }
            (ReportKind::Backdrop, None, None, Some(media_id)) => {
                let report = BackdropReport {
                    conversation_id: self.conversation_id,
                    media_id,
                    reason: self.reason,
                    comment: self.comment,
                };
                validate(&report)?;
                Ok(Filing::Backdrop(report))
            }
            _ => Err(ChatError::Invalid),
        }
    }
}

pub fn validate(report: &BackdropReport) -> ChatResult<()> {
    let comment_fits = report
        .comment
        .as_ref()
        .is_none_or(|comment| comment.chars().count() <= COMMENT_CHARS_MAX);
    if !REASONS.contains(&report.reason.as_str()) || !comment_fits {
        return Err(ChatError::Invalid);
    }
    Ok(())
}

pub async fn file_backdrop(
    chat: &ChatState,
    keys: &dyn ChatKeys,
    reporter: &str,
    input: &BackdropReport,
) -> ChatResult<FiledReport> {
    let conversation = input.conversation_id;
    let media = input.media_id;
    let found = sqlx::query_file!("queries/chat/reports/conversation.sql", conversation)
        .fetch_optional(&chat.pg)
        .await?
        .filter(|row| row.live)
        .ok_or(ChatError::NotFound)?;
    if found.kind != "room" {
        return Err(ChatError::Invalid);
    }
    let room = room_of_chat(chat, conversation)
        .await?
        .ok_or(ChatError::NotMember)?;
    if !room.is_member(reporter) {
        return Err(ChatError::NotMember);
    }
    if room.is_host(reporter) {
        return Err(ChatError::Invalid);
    }
    if let Some(found) = duplicate(&chat.pg, conversation, media, reporter).await? {
        return answer_again(chat, &room, input, found).await;
    }
    if room.image_shown() != Some(media) {
        return Err(ChatError::MediaMissing { ids: vec![media] });
    }
    let meta = meta_of(chat, conversation, &room.host_id).await?;
    if meta.is_empty() {
        return Err(ChatError::MediaMissing { ids: vec![media] });
    }
    let capture = evidence::capture(chat, keys, conversation, (0, 0), &meta).await?;
    users::touch(chat, reporter).await?;
    let weight = reporter_weight(chat, reporter).await?;
    let id = Uuid::now_v7();
    let mut tx = chat.pg.begin().await?;
    sqlx::query_file_scalar!("queries/chat/reports/lock_reporter.sql", media, reporter)
        .fetch_one(&mut *tx)
        .await?;
    if let Some(found) = duplicate(&mut *tx, conversation, media, reporter).await? {
        drop(tx);
        return answer_again(chat, &room, input, found).await;
    }
    sqlx::query_file!(
        "queries/chat/reports/insert_backdrop.sql",
        id,
        reporter,
        room.host_id,
        conversation,
        input.reason,
        input.comment.as_deref().map(str::trim).unwrap_or_default(),
        weight,
        capture.sealed,
        i16::try_from(capture.version).map_err(|_| ChatError::Internal)?,
        media
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query_file!(
        "queries/chat/reports/hold_backdrop.sql",
        media,
        id,
        HOLD_DAYS
    )
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    metrics::counter!("chat_backdrop_reports_total", "reason" => reason_label(&input.reason))
        .increment(1);
    if room.public && weight > 0.0 {
        crowd_clear(chat, conversation, media).await;
    }
    Ok(FiledReport {
        id,
        state: "open",
        duplicate: false,
    })
}

struct Filed {
    id: Uuid,
    weight: f32,
    state: String,
}

fn state_name(state: &str) -> &'static str {
    match state {
        "actioned" => "actioned",
        "rejected" => "rejected",
        _ => "open",
    }
}

async fn duplicate(
    executor: impl PgExecutor<'_>,
    conversation: Uuid,
    media: Uuid,
    reporter: &str,
) -> ChatResult<Option<Filed>> {
    Ok(sqlx::query_file_as!(
        Filed,
        "queries/chat/reports/backdrop_duplicate.sql",
        conversation,
        media,
        reporter
    )
    .fetch_optional(executor)
    .await?)
}

async fn answer_again(
    chat: &ChatState,
    room: &Room,
    input: &BackdropReport,
    found: Filed,
) -> ChatResult<FiledReport> {
    let media = input.media_id;
    if room.public && found.weight > 0.0 && room.image_shown() == Some(media) {
        crowd_clear(chat, input.conversation_id, media).await;
    }
    Ok(FiledReport {
        id: found.id,
        state: state_name(&found.state),
        duplicate: true,
    })
}

async fn meta_of(
    chat: &ChatState,
    conversation: Uuid,
    host: &str,
) -> ChatResult<Vec<StoredMessage>> {
    let row = sqlx::query_file!("queries/chat/reports/backdrop_meta.sql", conversation)
        .fetch_optional(&chat.pg)
        .await?;
    let Some((Some(body), Some(epoch))) = row.map(|row| (row.meta_ct, row.meta_epoch)) else {
        return Ok(Vec::new());
    };
    Ok(vec![StoredMessage {
        seq: 0,
        client_id: Uuid::nil(),
        sender_user: host.to_owned(),
        sender_device: None,
        class: "meta".to_owned(),
        epoch,
        target_seq: None,
        body,
        created_at: Utc::now(),
    }])
}

async fn crowd_clear(chat: &ChatState, conversation: Uuid, media: Uuid) {
    match cleared_by_reports(chat, conversation, media).await {
        Ok(true) => {
            metrics::counter!("chat_backdrops_cleared_total", "by" => "reports").increment(1);
        }
        Ok(false) => {}
        Err(error) => {
            metrics::counter!("chat_backdrop_clear_failures_total").increment(1);
            tracing::warn!(
                %conversation,
                code = error.code(),
                "a reported room backdrop clears on the next report"
            );
        }
    }
}

async fn cleared_by_reports(chat: &ChatState, conversation: Uuid, media: Uuid) -> ChatResult<bool> {
    let reporters = sqlx::query_file_scalar!(
        "queries/chat/reports/backdrop_reporters.sql",
        conversation,
        media,
        CLEAR_WINDOW_SECS
    )
    .fetch_one(&chat.pg)
    .await?;
    if reporters < CLEAR_REPORTERS {
        return Ok(false);
    }
    let until = Utc::now().timestamp_millis() + IMAGE_COOLDOWN_MS;
    backdrop::clear(chat, conversation, media, Some(until)).await
}
