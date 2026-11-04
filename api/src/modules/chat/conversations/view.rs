use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;

use crate::modules::chat::conversations::keys::peer_of;
use crate::modules::chat::members::access::GroupSettings;
use crate::modules::chat::{ChatError, ChatResult};

const PAGE_FRAME: usize = r#"{"items":[],"next_cursor":""}"#.len() + UUID_TEXT_LEN;
const UUID_TEXT_LEN: usize = 36;

#[derive(Clone, Debug)]
pub struct ListedRow {
    pub id: Uuid,
    pub kind: String,
    pub dm_key: Option<String>,
    pub room_ref: Option<String>,
    pub created_by: String,
    pub created_at: DateTime<Utc>,
    pub last_seq: i64,
    pub last_at: DateTime<Utc>,
    pub key_epoch: i32,
    pub meta_ct: Option<Vec<u8>>,
    pub meta_epoch: Option<i32>,
    pub settings: Value,
    pub member_count: i32,
    pub ttl_s: Option<i32>,
    pub role: String,
    pub state: String,
    pub accepted: bool,
    pub joined_seq: i64,
    pub joined_epoch: i32,
    pub last_read_seq: i64,
    pub muted_until: Option<DateTime<Utc>>,
    pub notify: String,
    pub pinned_at: Option<DateTime<Utc>>,
    pub archived: bool,
    pub hidden_before_seq: i64,
    pub invited_by: Option<String>,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct ConversationView {
    pub id: Uuid,
    pub kind: String,
    pub peer: Option<String>,
    pub created_by: String,
    pub created_at: i64,
    pub last_seq: i64,
    pub last_at: i64,
    pub key_epoch: i32,
    #[serde(with = "crate::modules::chat::b64::option")]
    pub meta_ct: Option<Vec<u8>>,
    pub meta_epoch: Option<i32>,
    pub settings: GroupSettings,
    pub member_count: i32,
    pub ttl_s: Option<i32>,
    pub room_ref: Option<String>,
    pub me: MembershipView,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct MembershipView {
    pub role: String,
    pub state: String,
    pub accepted: bool,
    pub joined_seq: i64,
    pub joined_epoch: i32,
    pub last_read_seq: i64,
    pub muted_until: Option<i64>,
    pub notify: String,
    pub pinned_at: Option<i64>,
    pub archived: bool,
    pub hidden_before_seq: i64,
    pub invited_by: Option<String>,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct ConversationPage {
    pub items: Vec<ConversationView>,
    pub next_cursor: Option<String>,
}

impl ListedRow {
    pub fn view(self, user: &str) -> ConversationView {
        ConversationView {
            id: self.id,
            peer: match self.kind.as_str() {
                "direct" => self.dm_key.as_deref().and_then(|key| peer_of(key, user)),
                _ => None,
            },
            kind: self.kind,
            created_by: self.created_by,
            created_at: self.created_at.timestamp_millis(),
            last_seq: self.last_seq,
            last_at: self.last_at.timestamp_millis(),
            key_epoch: self.key_epoch,
            meta_ct: self.meta_ct,
            meta_epoch: self.meta_epoch,
            settings: GroupSettings::of(&self.settings),
            member_count: self.member_count,
            ttl_s: self.ttl_s,
            room_ref: self.room_ref,
            me: MembershipView {
                role: self.role,
                state: self.state,
                accepted: self.accepted,
                joined_seq: self.joined_seq,
                joined_epoch: self.joined_epoch,
                last_read_seq: self.last_read_seq,
                muted_until: self.muted_until.map(|at| at.timestamp_millis()),
                notify: self.notify,
                pinned_at: self.pinned_at.map(|at| at.timestamp_millis()),
                archived: self.archived,
                hidden_before_seq: self.hidden_before_seq,
                invited_by: self.invited_by,
            },
        }
    }
}

pub async fn one(pg: &PgPool, conversation: Uuid, user: &str) -> ChatResult<ConversationView> {
    sqlx::query_file_as!(
        ListedRow,
        "queries/chat/conversations/one.sql",
        conversation,
        user
    )
    .fetch_optional(pg)
    .await?
    .map(|row| row.view(user))
    .ok_or(ChatError::NotMember)
}

pub async fn rows_after(
    pg: &PgPool,
    user: &str,
    cursor: Uuid,
    limit: i64,
) -> ChatResult<Vec<ListedRow>> {
    Ok(sqlx::query_file_as!(
        ListedRow,
        "queries/chat/conversations/page.sql",
        user,
        cursor,
        limit
    )
    .fetch_all(pg)
    .await?)
}

pub fn fit(rows: Vec<ListedRow>, user: &str, limit: usize, byte_cap: usize) -> ConversationPage {
    let mut more = rows.len() > limit;
    let mut used = PAGE_FRAME;
    let mut items = Vec::with_capacity(rows.len().min(limit));
    for row in rows.into_iter().take(limit) {
        let view = row.view(user);
        let size = serde_json::to_vec(&view).map_or(usize::MAX, |json| json.len());
        let after = used
            .saturating_add(size)
            .saturating_add(usize::from(!items.is_empty()));
        if !items.is_empty() && after > byte_cap {
            more = true;
            break;
        }
        used = after;
        items.push(view);
    }
    let next_cursor = more
        .then(|| items.last().map(|view| view.id.to_string()))
        .flatten();
    ConversationPage { items, next_cursor }
}
