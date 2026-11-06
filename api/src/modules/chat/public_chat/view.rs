use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::Value;

use crate::modules::chat::conversations::view::{ConversationView, MembershipView};
use crate::modules::chat::members::access::GroupSettings;
use crate::modules::chat::public_chat::{ID, watchers};
use crate::modules::chat::{ChatError, ChatResult, ChatState};

pub struct Row {
    pub id: uuid::Uuid,
    pub created_by: String,
    pub created_at: DateTime<Utc>,
    pub last_seq: i64,
    pub last_at: DateTime<Utc>,
    pub key_epoch: i32,
    pub settings: Value,
    pub role: Option<String>,
    pub state: Option<String>,
    pub last_read_seq: Option<i64>,
    pub muted_until: Option<DateTime<Utc>>,
    pub notify: Option<String>,
    pub hidden_before_seq: Option<i64>,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct PublicView {
    pub conversation: ConversationView,
    pub online: u32,
}

impl Row {
    pub fn joined(&self) -> bool {
        self.state.is_some()
    }

    fn into_view(self, online: u32) -> ConversationView {
        let pinned = self.created_at.timestamp_millis();
        ConversationView {
            id: self.id,
            kind: "public".to_owned(),
            peer: None,
            created_by: self.created_by,
            created_at: pinned,
            last_seq: self.last_seq,
            last_at: self.last_at.timestamp_millis(),
            key_epoch: self.key_epoch,
            meta_ct: None,
            meta_epoch: None,
            settings: GroupSettings::of(&self.settings),
            member_count: i32::try_from(online).unwrap_or(i32::MAX),
            ttl_s: None,
            room_ref: None,
            me: MembershipView {
                role: self.role.unwrap_or_else(|| "member".to_owned()),
                state: self.state.unwrap_or_else(|| "active".to_owned()),
                accepted: true,
                joined_seq: 0,
                joined_epoch: 0,
                last_read_seq: self.last_read_seq.unwrap_or(0),
                muted_until: self.muted_until.map(|at| at.timestamp_millis()),
                notify: self.notify.unwrap_or_else(|| "mentions".to_owned()),
                pinned_at: Some(pinned),
                archived: false,
                hidden_before_seq: self.hidden_before_seq.unwrap_or(0),
                invited_by: None,
            },
        }
    }
}

pub async fn row(chat: &ChatState, user_id: &str) -> ChatResult<Row> {
    sqlx::query_file_as!(Row, "queries/chat/public/view.sql", ID, user_id)
        .fetch_optional(&chat.pg)
        .await?
        .ok_or(ChatError::NotFound)
}

pub async fn of(chat: &ChatState, user_id: &str) -> ChatResult<ConversationView> {
    let row = row(chat, user_id).await?;
    Ok(row.into_view(watchers::count(chat).await?))
}

pub async fn opened(chat: &ChatState, row: Row) -> ChatResult<PublicView> {
    let online = watchers::count(chat).await?;
    Ok(PublicView {
        conversation: row.into_view(online),
        online,
    })
}
