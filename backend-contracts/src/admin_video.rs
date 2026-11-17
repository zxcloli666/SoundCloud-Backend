use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const ADMIN_VIDEO_SUBJECT: &str = "admin.video.v1";
pub const ADMIN_VIDEO_QUEUE: &str = "jobs-admin-video-v1";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum VideoSource {
    Youtube,
    Spotify,
    Yandex,
}

impl VideoSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Youtube => "youtube",
            Self::Spotify => "spotify",
            Self::Yandex => "yandex",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "youtube" => Some(Self::Youtube),
            "spotify" => Some(Self::Spotify),
            "yandex" => Some(Self::Yandex),
            _ => None,
        }
    }
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", content = "args", rename_all = "snake_case")]
pub enum AdminVideoCommand {
    List,
    Save {
        source: VideoSource,
        name: String,
        secret: String,
    },
    Check {
        name: String,
    },
}

impl fmt::Debug for AdminVideoCommand {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::List => formatter.write_str("List"),
            Self::Save { source, name, .. } => formatter
                .debug_struct("Save")
                .field("source", source)
                .field("name", name)
                .field("secret", &"[redacted]")
                .finish(),
            Self::Check { name } => formatter.debug_struct("Check").field("name", name).finish(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AdminVideoRequest {
    pub command: AdminVideoCommand,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AdminVideoReply {
    pub status: u16,
    pub body: Value,
}
