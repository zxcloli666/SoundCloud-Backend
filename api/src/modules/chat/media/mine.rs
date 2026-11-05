use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::modules::chat::body::ChatQuery;
use crate::modules::chat::limits::{self, Bucket};
use crate::modules::chat::media::limits::{MINE_LIMIT_DEFAULT, MINE_LIMIT_MAX};
use crate::modules::chat::media::wire::{
    MediaKind, MineItem, MineOrder, MinePage, MineQuery, MineStatus,
};
use crate::modules::chat::quota::unsigned;
use crate::modules::chat::{ChatCaller, ChatError, ChatResult, ChatState};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Cursor {
    order: MineOrder,
    value: i64,
    id: Uuid,
}

impl Cursor {
    fn encode(self) -> String {
        URL_SAFE_NO_PAD.encode(format!(
            "{}:{}:{}",
            order_tag(self.order),
            self.value,
            self.id
        ))
    }

    fn decode(raw: &str, order: MineOrder) -> ChatResult<Self> {
        let bytes = URL_SAFE_NO_PAD
            .decode(raw)
            .map_err(|_| ChatError::Invalid)?;
        let text = String::from_utf8(bytes).map_err(|_| ChatError::Invalid)?;
        let mut parts = text.splitn(3, ':');
        let (Some(tag), Some(value), Some(id)) = (parts.next(), parts.next(), parts.next()) else {
            return Err(ChatError::Invalid);
        };
        let value: i64 = value.parse().map_err(|_| ChatError::Invalid)?;
        let timed = order != MineOrder::Big;
        if tag != order_tag(order) || (timed && DateTime::from_timestamp_micros(value).is_none()) {
            return Err(ChatError::Invalid);
        }
        Ok(Self {
            order,
            value,
            id: id.parse().map_err(|_| ChatError::Invalid)?,
        })
    }
}

fn order_tag(order: MineOrder) -> &'static str {
    match order {
        MineOrder::New => "new",
        MineOrder::Old => "old",
        MineOrder::Big => "big",
    }
}

struct Row {
    id: Uuid,
    kind: String,
    origin_conversation: Uuid,
    thumb_id: Option<Uuid>,
    bytes: i64,
    status: String,
    chats: i32,
    created_at: DateTime<Utc>,
}

pub async fn media_mine(
    caller: ChatCaller,
    State(chat): State<Arc<ChatState>>,
    ChatQuery(query): ChatQuery<MineQuery>,
) -> ChatResult<Json<MinePage>> {
    limits::admit(&chat, Bucket::MediaMine, &caller.user_id).await?;
    if query
        .limit
        .is_some_and(|limit| limit == 0 || limit > MINE_LIMIT_MAX)
    {
        return Err(ChatError::Invalid);
    }
    list_rows(&chat, &caller, &query).await.map(Json)
}

async fn list_rows(
    chat: &ChatState,
    caller: &ChatCaller,
    query: &MineQuery,
) -> ChatResult<MinePage> {
    let cursor = query
        .cursor
        .as_deref()
        .map(|raw| Cursor::decode(raw, query.order))
        .transpose()?;
    let limit = query.limit.unwrap_or(MINE_LIMIT_DEFAULT);
    let mut rows = fetch(chat, &caller.user_id, query, cursor, i64::from(limit) + 1).await?;
    let more = rows.len() > limit as usize;
    rows.truncate(limit as usize);
    let next_cursor = more
        .then(|| rows.last().map(|row| cursor_after(row, query.order)))
        .flatten();
    let items = rows.into_iter().map(item).collect::<ChatResult<_>>()?;
    Ok(MinePage { items, next_cursor })
}

fn cursor_after(row: &Row, order: MineOrder) -> String {
    let value = match order {
        MineOrder::Big => row.bytes,
        MineOrder::New | MineOrder::Old => row.created_at.timestamp_micros(),
    };
    Cursor {
        order,
        value,
        id: row.id,
    }
    .encode()
}

fn item(row: Row) -> ChatResult<MineItem> {
    let status = match row.status.as_str() {
        "uploaded" => MineStatus::Uploaded,
        "attached" => MineStatus::Attached,
        "lost" => MineStatus::Lost,
        _ => return Err(ChatError::Internal),
    };
    Ok(MineItem {
        id: row.id,
        kind: MediaKind::parse(&row.kind).ok_or(ChatError::Internal)?,
        conversation_id: row.origin_conversation,
        thumb_id: row.thumb_id,
        bytes: unsigned(row.bytes),
        status,
        chats: u32::try_from(row.chats).unwrap_or(0),
        created_at: row.created_at.timestamp_millis(),
    })
}

async fn fetch(
    chat: &ChatState,
    user: &str,
    query: &MineQuery,
    cursor: Option<Cursor>,
    limit: i64,
) -> ChatResult<Vec<Row>> {
    let origin = query.conversation_id;
    let after_id = cursor.map(|cursor| cursor.id);
    let after_at = cursor.and_then(|cursor| DateTime::from_timestamp_micros(cursor.value));
    let after_bytes = cursor.map(|cursor| cursor.value);
    let rows = match query.order {
        MineOrder::New => {
            sqlx::query_file_as!(
                Row,
                "queries/chat/media/mine_new.sql",
                user,
                origin,
                after_at,
                after_id,
                limit
            )
            .fetch_all(&chat.pg)
            .await?
        }
        MineOrder::Old => {
            sqlx::query_file_as!(
                Row,
                "queries/chat/media/mine_old.sql",
                user,
                origin,
                after_at,
                after_id,
                limit
            )
            .fetch_all(&chat.pg)
            .await?
        }
        MineOrder::Big => {
            sqlx::query_file_as!(
                Row,
                "queries/chat/media/mine_big.sql",
                user,
                origin,
                after_bytes,
                after_id,
                limit
            )
            .fetch_all(&chat.pg)
            .await?
        }
    };
    Ok(rows)
}

#[cfg(test)]
#[path = "../tests/media_listing_tests.rs"]
mod tests;
