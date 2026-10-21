use crate::modules::chat::moderation::decide::REPEAT_WINDOW_DAYS;
use crate::modules::chat::reports::ReportRow;
use crate::modules::chat::{ChatResult, ChatState};

pub const HISTORY_DAYS: i32 = 90;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ReporterHistory {
    pub filed: i64,
    pub upheld: i64,
    pub rejected: i64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TargetHistory {
    pub reported: i64,
    pub upheld: i64,
    pub restricted: bool,
}

pub async fn reporter(chat: &ChatState, user_id: &str) -> ChatResult<ReporterHistory> {
    let row = sqlx::query_file!(
        "queries/chat/reports/reporter_history.sql",
        user_id,
        HISTORY_DAYS
    )
    .fetch_one(&chat.pg)
    .await?;
    Ok(ReporterHistory {
        filed: row.filed,
        upheld: row.upheld,
        rejected: row.rejected,
    })
}

pub async fn target(chat: &ChatState, user_id: &str) -> ChatResult<TargetHistory> {
    let row = sqlx::query_file!(
        "queries/chat/reports/target_history.sql",
        user_id,
        HISTORY_DAYS
    )
    .fetch_one(&chat.pg)
    .await?;
    Ok(TargetHistory {
        reported: row.reported,
        upheld: row.upheld,
        restricted: row.restricted,
    })
}

pub async fn repeat_offender(chat: &ChatState, report: &ReportRow) -> ChatResult<bool> {
    Ok(sqlx::query_file_scalar!(
        "queries/chat/reports/repeat.sql",
        report.target_user,
        report.id,
        REPEAT_WINDOW_DAYS as i32,
        report.conversation_id,
        report.seq_from,
        report.seq_to
    )
    .fetch_one(&chat.pg)
    .await?)
}
