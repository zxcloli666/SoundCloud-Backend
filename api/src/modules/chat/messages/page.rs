use sqlx::PgPool;

use crate::modules::chat::members::access::Access;
use crate::modules::chat::messages::{
    HistoryPage, HistoryQuery, PAGE_BYTES_MAX, PAGE_BYTES_MAX_PRO, PAGE_ROWS_MAX,
};
use crate::modules::chat::model::{MessageRow, MessageView, Tier};
use crate::modules::chat::{ChatError, ChatResult};

pub const PAGE_ROWS_DEFAULT: usize = 50;

const PAGE_FRAME: usize = r#"{"messages":[],"more":false}"#.len();

pub fn byte_cap(tier: Tier, max_bytes: u32) -> usize {
    let tier_cap = match tier {
        Tier::Pro => PAGE_BYTES_MAX_PRO,
        _ => PAGE_BYTES_MAX,
    };
    match usize::try_from(max_bytes) {
        Ok(asked) if asked > 0 => tier_cap.min(asked),
        _ => tier_cap,
    }
}

pub fn rows_limit(asked: Option<u32>) -> usize {
    asked
        .and_then(|limit| usize::try_from(limit).ok())
        .filter(|limit| *limit > 0)
        .map_or(PAGE_ROWS_DEFAULT, |limit| limit.min(PAGE_ROWS_MAX))
}

pub fn fit(rows: Vec<MessageRow>, limit: usize, byte_cap: usize) -> HistoryPage {
    let mut more = rows.len() > limit;
    let mut used = PAGE_FRAME;
    let mut messages = Vec::with_capacity(rows.len().min(limit));
    for row in rows.iter().take(limit) {
        let view = MessageView::from(row);
        let size = serde_json::to_vec(&view).map_or(usize::MAX, |json| json.len());
        let after = used
            .saturating_add(size)
            .saturating_add(usize::from(!messages.is_empty()));
        if !messages.is_empty() && after > byte_cap {
            more = true;
            break;
        }
        used = after;
        messages.push(view);
    }
    HistoryPage { messages, more }
}

pub async fn read(
    pg: &PgPool,
    access: &Access,
    query: &HistoryQuery,
    byte_cap: usize,
) -> ChatResult<HistoryPage> {
    let limit = rows_limit(query.limit);
    let fetch = i64::try_from(limit + 1).map_err(|_| ChatError::Invalid)?;
    let floor = access.history_floor();
    let conversation = access.conversation;
    match (query.after, query.before) {
        (Some(_), Some(_)) => Err(ChatError::Invalid),
        (Some(after), None) => {
            let rows = sqlx::query_file_as!(
                MessageRow,
                "queries/chat/messages/after.sql",
                conversation,
                after.max(floor),
                fetch
            )
            .fetch_all(pg)
            .await?;
            Ok(fit(rows, limit, byte_cap))
        }
        (None, before) => {
            let mut rows = sqlx::query_file_as!(
                MessageRow,
                "queries/chat/messages/before.sql",
                conversation,
                before.unwrap_or(i64::MAX),
                floor,
                fetch
            )
            .fetch_all(pg)
            .await?;
            let more = rows.len() > limit;
            rows.truncate(limit);
            let newest = rows.len();
            let mut page = fit(rows, newest, byte_cap);
            page.more |= more;
            page.messages.reverse();
            Ok(page)
        }
    }
}

#[cfg(test)]
#[path = "../tests/history_page_tests.rs"]
mod tests;
