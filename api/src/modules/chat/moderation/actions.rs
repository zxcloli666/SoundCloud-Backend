use chrono::{Duration, Utc};
use uuid::Uuid;

use crate::modules::chat::moderation::decide::Ban;
use crate::modules::chat::reports::EVIDENCE_KEEP_DAYS;
use crate::modules::chat::sweeps::hooks;
use crate::modules::chat::users::{self, Restriction, Scope};
use crate::modules::chat::{ChatResult, ChatState};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Closing {
    Actioned,
    Rejected,
}

impl Closing {
    pub fn as_str(self) -> &'static str {
        match self {
            Closing::Actioned => "actioned",
            Closing::Rejected => "rejected",
        }
    }
}

pub async fn remove_messages(
    chat: &ChatState,
    conversation: Uuid,
    sender: &str,
    seqs: &[i64],
) -> ChatResult<Vec<i64>> {
    let removed = sqlx::query_file_scalar!(
        "queries/chat/reports/remove_messages.sql",
        conversation,
        sender,
        seqs
    )
    .fetch_all(&chat.pg)
    .await?;
    if !removed.is_empty() {
        chat.hot.forget(conversation);
    }
    Ok(removed)
}

pub async fn announce_removed(chat: &ChatState, conversation: Uuid, removed: &[i64]) {
    if removed.is_empty() {
        return;
    }
    if let Err(error) = announce(chat, conversation, removed).await {
        tracing::warn!(
            code = error.code(),
            "removed chat messages were not announced"
        );
    }
}

async fn announce(chat: &ChatState, conversation: Uuid, removed: &[i64]) -> ChatResult<()> {
    let members = sqlx::query_file_scalar!("queries/chat/reports/live_members.sql", conversation)
        .fetch_all(&chat.pg)
        .await?;
    hooks::messages_gone(chat, conversation, removed, &members).await
}

pub async fn ban(chat: &ChatState, user_id: &str, ban: Ban) -> ChatResult<Option<Restriction>> {
    let until = match ban {
        Ban::None => return Ok(None),
        Ban::Days(days) => Utc::now() + Duration::days(days),
        Ban::Permanent => users::permanent(),
    };
    Ok(Some(
        users::restrict(chat, user_id, until, &[Scope::All]).await?,
    ))
}

pub async fn close_open(
    chat: &ChatState,
    report: Uuid,
    closing: Closing,
    by: &str,
) -> ChatResult<bool> {
    let closed = sqlx::query_file_scalar!(
        "queries/chat/reports/close_open.sql",
        report,
        closing.as_str(),
        by,
        EVIDENCE_KEEP_DAYS as i32
    )
    .fetch_optional(&chat.pg)
    .await?;
    Ok(closed.is_some())
}

pub async fn mark_judged(chat: &ChatState, report: Uuid, by: &str) -> ChatResult<()> {
    sqlx::query_file!(
        "queries/chat/reports/mark_judged.sql",
        report,
        by,
        EVIDENCE_KEEP_DAYS as i32
    )
    .execute(&chat.pg)
    .await?;
    Ok(())
}
