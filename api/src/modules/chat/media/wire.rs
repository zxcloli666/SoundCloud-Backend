use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::modules::chat::media::limits::{
    AVATAR_MAX_BYTES, BACKDROP_MAX_BYTES, BLOCK, IMAGE_MAX_BYTES, SELECTION_MAX, THUMB_MAX_BYTES,
    URLS_MAX,
};
use crate::modules::chat::{ChatError, ChatResult};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MediaKind {
    Image,
    Avatar,
    Backdrop,
}

impl MediaKind {
    pub fn as_str(self) -> &'static str {
        match self {
            MediaKind::Image => "image",
            MediaKind::Avatar => "avatar",
            MediaKind::Backdrop => "backdrop",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "image" => Some(MediaKind::Image),
            "avatar" => Some(MediaKind::Avatar),
            "backdrop" => Some(MediaKind::Backdrop),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReserveBody {
    pub upload_id: Uuid,
    pub conversation_id: Uuid,
    pub kind: MediaKind,
    pub bytes: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thumb_bytes: Option<u64>,
}

fn padded(bytes: u64, max: u64) -> bool {
    bytes > 0 && bytes <= max && bytes.is_multiple_of(BLOCK)
}

impl ReserveBody {
    pub fn validate(&self) -> ChatResult<()> {
        let fits = match (self.kind, self.thumb_bytes) {
            (MediaKind::Image, Some(thumb)) => {
                padded(self.bytes, IMAGE_MAX_BYTES) && padded(thumb, THUMB_MAX_BYTES)
            }
            (MediaKind::Avatar, None) => padded(self.bytes, AVATAR_MAX_BYTES),
            (MediaKind::Backdrop, None) => padded(self.bytes, BACKDROP_MAX_BYTES),
            _ => false,
        };
        if fits {
            Ok(())
        } else {
            Err(ChatError::Invalid)
        }
    }

    pub fn total(&self) -> u64 {
        self.bytes + self.thumb_bytes.unwrap_or(0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PresignedPut {
    pub url: String,
    pub headers: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReservedThumb {
    pub id: Uuid,
    pub put: PresignedPut,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReserveAnswer {
    pub id: Uuid,
    pub put: PresignedPut,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thumb: Option<ReservedThumb>,
    pub expires_in_s: u32,
    pub used: u64,
    pub quota: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommitAnswer {
    pub id: Uuid,
    pub bytes: u64,
    pub used: u64,
    pub quota: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UrlsBody {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversation_id: Option<Uuid>,
    pub ids: Vec<Uuid>,
}

impl UrlsBody {
    pub fn validate(&self) -> ChatResult<()> {
        distinct(&self.ids, URLS_MAX)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum MediaUrl {
    Ok { id: Uuid, url: String },
    Deleted { id: Uuid },
    Lost { id: Uuid },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UrlsAnswer {
    pub expires_in_s: u32,
    pub items: Vec<MediaUrl>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConversationUsage {
    pub conversation_id: Uuid,
    pub bytes: u64,
    pub files: u32,
    pub gone: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UsageAnswer {
    pub bytes: u64,
    pub reserved: u64,
    pub quota: u64,
    pub files: u32,
    pub conversations: Vec<ConversationUsage>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MineOrder {
    #[default]
    New,
    Old,
    Big,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct MineQuery {
    #[serde(default)]
    pub order: MineOrder,
    pub conversation_id: Option<Uuid>,
    pub cursor: Option<String>,
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MineStatus {
    Uploaded,
    Attached,
    Lost,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MineItem {
    pub id: Uuid,
    pub kind: MediaKind,
    pub conversation_id: Uuid,
    pub thumb_id: Option<Uuid>,
    pub bytes: u64,
    pub status: MineStatus,
    pub chats: u32,
    pub created_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MinePage {
    pub items: Vec<MineItem>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Selection {
    #[serde(default)]
    pub ids: Vec<Uuid>,
    #[serde(default)]
    pub all: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Picked {
    Ids(Vec<Uuid>),
    All,
}

impl Picked {
    pub fn parts(&self) -> (&[Uuid], bool) {
        match self {
            Picked::Ids(ids) => (ids, false),
            Picked::All => (&[], true),
        }
    }
}

impl Selection {
    pub fn pick(&self) -> ChatResult<Picked> {
        match (self.ids.is_empty(), self.all) {
            (false, false) => deduped(&self.ids).map(Picked::Ids),
            (true, true) => Ok(Picked::All),
            _ => Err(ChatError::Invalid),
        }
    }
}

fn deduped(ids: &[Uuid]) -> ChatResult<Vec<Uuid>> {
    if ids.len() > SELECTION_MAX {
        return Err(ChatError::Invalid);
    }
    let mut seen = std::collections::HashSet::with_capacity(ids.len());
    Ok(ids.iter().copied().filter(|id| seen.insert(*id)).collect())
}

fn distinct(ids: &[Uuid], max: usize) -> ChatResult<()> {
    let unique: std::collections::HashSet<&Uuid> = ids.iter().collect();
    if ids.is_empty() || ids.len() > max || unique.len() != ids.len() {
        return Err(ChatError::Invalid);
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeleteAnswer {
    pub deleted: u32,
    pub freed: u64,
    pub used: u64,
    pub quota: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RestoreAnswer {
    pub restored: u32,
    pub missed: u32,
    pub used: u64,
    pub quota: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MediaConfigView {
    pub uploads: bool,
    pub star: bool,
    pub quota: u64,
    pub used: u64,
    pub limits: MediaLimitsView,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MediaLimitsView {
    pub image_max_bytes: u32,
    pub thumb_max_bytes: u32,
    pub avatar_max_bytes: u32,
    pub album_max: u32,
    pub room_album_max: u32,
    pub restore_window_s: u32,
}

#[cfg(test)]
#[path = "../tests/media_wire_tests.rs"]
mod tests;
