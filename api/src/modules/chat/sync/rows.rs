use std::collections::HashMap;

use uuid::Uuid;

use crate::modules::chat::model::MessageRow;
use crate::modules::chat::{ChatResult, ChatState};

pub const ROWS_PER_CONVERSATION: i64 = 200;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Fetch {
    pub conversation: Uuid,
    pub after: i64,
}

pub async fn fetch(
    chat: &ChatState,
    wanted: &[Fetch],
    budget: usize,
) -> ChatResult<HashMap<Uuid, Vec<MessageRow>>> {
    let mut found = HashMap::new();
    let mut missed = Vec::new();
    for fetch in wanted {
        match chat.hot.after(fetch.conversation, fetch.after) {
            Some(rows) => {
                found.insert(fetch.conversation, rows);
            }
            None => missed.push(*fetch),
        }
    }
    if missed.is_empty() {
        return Ok(found);
    }
    let conversations: Vec<Uuid> = missed.iter().map(|fetch| fetch.conversation).collect();
    let afters: Vec<i64> = missed.iter().map(|fetch| fetch.after).collect();
    let rows = sqlx::query_file_as!(
        MessageRow,
        "queries/chat/sync/messages_after.sql",
        &conversations,
        &afters,
        ROWS_PER_CONVERSATION,
        i64::try_from(budget).unwrap_or(i64::MAX)
    )
    .fetch_all(&chat.pg)
    .await?;
    for row in rows {
        found
            .entry(row.conversation_id)
            .or_insert_with(Vec::new)
            .push(row);
    }
    Ok(found)
}
