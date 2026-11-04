use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub use sc_chat_wire::proto::Tier;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConversationKind {
    Direct,
    Group,
    Room,
    #[serde(rename = "self")]
    Saved,
}

impl ConversationKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ConversationKind::Direct => "direct",
            ConversationKind::Group => "group",
            ConversationKind::Room => "room",
            ConversationKind::Saved => "self",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemberRole {
    Owner,
    Admin,
    Member,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemberState {
    Active,
    Invited,
    Left,
    Kicked,
    Banned,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeviceStatus {
    Active,
    Revoked,
}

pub const DISAPPEARING_TTLS: [i32; 3] = [3_600, 86_400, 604_800];
pub const GROUP_MEMBERS_MAX: usize = 200;
pub const DEVICES_MAX: usize = 10;

#[derive(Clone, Debug)]
pub struct DeviceRow {
    pub id: Uuid,
    pub user_id: String,
    pub session_id: Uuid,
    pub status: String,
    pub auth_pub: Vec<u8>,
    pub box_pub: Vec<u8>,
    pub platform: String,
    pub name: String,
    pub app_version: String,
    pub caps: Vec<String>,
    pub tier: String,
    pub created_at: DateTime<Utc>,
    pub history_unlocked_at: Option<DateTime<Utc>>,
    pub last_seen_at: DateTime<Utc>,
    pub revoked_at: Option<DateTime<Utc>>,
}

#[derive(Clone, Debug)]
pub struct MessageRow {
    pub conversation_id: Uuid,
    pub seq: i64,
    pub client_id: Uuid,
    pub sender_user: String,
    pub sender_device: Option<Uuid>,
    pub class: String,
    pub epoch: i32,
    pub target_seq: Option<i64>,
    pub body: Option<Vec<u8>>,
    pub created_at: DateTime<Utc>,
    pub expires_at: Option<DateTime<Utc>>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct MessageView {
    pub seq: i64,
    pub client_id: Uuid,
    pub sender: String,
    pub sender_device: Option<Uuid>,
    pub class: String,
    pub epoch: i32,
    pub target_seq: Option<i64>,
    #[serde(with = "crate::modules::chat::b64::option")]
    pub body: Option<Vec<u8>>,
    pub ts: i64,
    pub expires_at: Option<i64>,
}

impl From<&MessageRow> for MessageView {
    fn from(row: &MessageRow) -> Self {
        Self {
            seq: row.seq,
            client_id: row.client_id,
            sender: row.sender_user.clone(),
            sender_device: row.sender_device,
            class: row.class.clone(),
            epoch: row.epoch,
            target_seq: row.target_seq,
            body: row.body.clone(),
            ts: row.created_at.timestamp_millis(),
            expires_at: row.expires_at.map(|at| at.timestamp_millis()),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct DeviceView {
    pub id: Uuid,
    pub platform: String,
    pub name: String,
    pub app_version: String,
    pub tier: String,
    pub status: DeviceStatus,
    pub created_at: i64,
    pub last_seen_at: i64,
    pub history_unlocked: bool,
    pub current: bool,
}

#[cfg(test)]
#[path = "tests/model_tests.rs"]
mod tests;
