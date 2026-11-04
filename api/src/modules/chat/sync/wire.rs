use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::modules::chat::model::{DeviceView, MessageView};
use crate::modules::chat::{ChatError, ChatResult};

pub const HOLD_MAX_MS: u64 = 20_000;
pub const HOLD_MAX_PRO_MS: u64 = 15_000;
pub const KNOWN_MAX: usize = 64;
pub const MARKS_MAX: usize = 64;
pub const ANSWER_BYTES_MAX: u32 = 256 * 1024;
pub const ANSWER_BYTES_MAX_PRO: u32 = 24 * 1024;
pub const RETRY_AFTER_MS: &str = "retry-after-ms";

#[derive(Debug, Clone, Deserialize)]
pub struct SyncRequest {
    #[serde(default)]
    pub pos: u64,
    pub inc: Option<Uuid>,
    #[serde(default)]
    pub hold_ms: u64,
    pub max_bytes: u32,
    #[serde(default)]
    pub known: Vec<(Uuid, i64)>,
    pub focus: Option<Uuid>,
    #[serde(default)]
    pub read: Vec<(Uuid, i64)>,
    #[serde(default)]
    pub delivered: Vec<(Uuid, i64)>,
}

impl SyncRequest {
    pub fn validate(&self) -> ChatResult<()> {
        let Self {
            known,
            read,
            delivered,
            ..
        } = self;
        let too_many =
            known.len() > KNOWN_MAX || read.len() > MARKS_MAX || delivered.len() > MARKS_MAX;
        let negative = known
            .iter()
            .chain(read)
            .chain(delivered)
            .any(|(_, seq)| *seq < 0);
        if too_many || negative {
            return Err(ChatError::Invalid);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct SyncAnswer {
    pub pos: u64,
    pub inc: Uuid,
    pub reset: bool,
    pub convs: Vec<SyncConversation>,
    pub gone: Vec<Uuid>,
    pub gone_msgs: Vec<(Uuid, i64)>,
    pub devices: Vec<DeviceView>,
    pub requests_count: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct SyncConversation {
    pub id: Uuid,
    pub last_seq: i64,
    pub key_epoch: i32,
    pub messages: Vec<MessageView>,
    pub more: bool,
    pub marks: Vec<PeerMark>,
    pub typing: Vec<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct PeerMark {
    pub user_id: String,
    pub read_seq: i64,
    pub delivered_seq: i64,
}
