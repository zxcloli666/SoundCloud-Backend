use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

pub const COMMAND_SUBJECT: &str = "remote.cmd";
pub const PRESENCE_SUBJECT: &str = "remote.presence";
pub const REPLY_SUBJECT_PREFIX: &str = "remote.reply";
pub const WAIT_HOLD_SECS: u64 = 20;
pub const BUFFER_SECS: u64 = 60;
pub const LINK_CODE_DIGITS: usize = 6;
pub const LINK_CODE_TTL_SECS: i64 = 600;
pub const CONFIRM_HOLD_SECS: u64 = 60;
pub const MAX_DEVICES_PER_USER: usize = 3;
pub const MAX_REPLY_BYTES: usize = 8 * 1024;
pub const MAX_UTTERANCE_CHARS: usize = 300;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LinkProvider {
    Alice,
    Salut,
}

impl LinkProvider {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Alice => "alice",
            Self::Salut => "salut",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "alice" => Some(Self::Alice),
            "salut" => Some(Self::Salut),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommandKind {
    #[default]
    Utterance,
    LinkConfirm,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteCommand {
    pub id: Uuid,
    #[serde(default)]
    pub kind: CommandKind,
    pub seq: i64,
    pub user: String,
    pub source: String,
    pub text: String,
    pub created_at: i64,
    pub expires_at: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteReply {
    pub id: Uuid,
    pub ok: bool,
    #[serde(default)]
    pub speech: String,
    #[serde(default)]
    pub display: String,
    #[serde(default)]
    pub state: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WaitResponse {
    pub cursor: i64,
    pub commands: Vec<RemoteCommand>,
    pub active_device: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LinkCodeRequest {
    pub provider: LinkProvider,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LinkCodeResponse {
    pub code: String,
    pub expires_at: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LinkInfo {
    pub id: Uuid,
    pub provider: LinkProvider,
    pub created_at: i64,
    pub last_used_at: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LinksResponse {
    pub links: Vec<LinkInfo>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Presence {
    pub user: String,
    pub device: String,
    pub active: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReplyEnvelope {
    pub user: String,
    pub reply: RemoteReply,
}

pub fn reply_subject(id: Uuid) -> String {
    format!("{REPLY_SUBJECT_PREFIX}.{id}")
}

pub fn reply_subject_filter() -> String {
    format!("{REPLY_SUBJECT_PREFIX}.*")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_command_is_camel_case_on_the_wire() {
        let command = RemoteCommand {
            id: Uuid::nil(),
            kind: CommandKind::Utterance,
            seq: 7,
            user: "101".into(),
            source: "alice".into(),
            text: "включи следующий трек".into(),
            created_at: 1,
            expires_at: 2,
        };
        let json = serde_json::to_value(&command).unwrap();
        assert_eq!(json["expiresAt"], 2);
        assert_eq!(json["createdAt"], 1);
        let back: RemoteCommand = serde_json::from_value(json).unwrap();
        assert_eq!(back, command);
    }

    #[test]
    fn a_command_without_a_kind_is_an_utterance_and_a_link_confirm_is_snake_case() {
        let old: RemoteCommand = serde_json::from_str(
            r#"{"id":"00000000-0000-0000-0000-000000000000","seq":1,"user":"1","source":"alice","text":"x","createdAt":1,"expiresAt":2}"#,
        )
        .unwrap();
        assert_eq!(old.kind, CommandKind::Utterance);
        let json = serde_json::to_value(CommandKind::LinkConfirm).unwrap();
        assert_eq!(json, "link_confirm");
    }

    #[test]
    fn a_reply_without_optional_parts_still_parses() {
        let reply: RemoteReply =
            serde_json::from_str(r#"{"id":"00000000-0000-0000-0000-000000000000","ok":true}"#)
                .unwrap();
        assert!(reply.ok && reply.speech.is_empty() && reply.state.is_none());
    }

    #[test]
    fn providers_round_trip() {
        for provider in [LinkProvider::Alice, LinkProvider::Salut] {
            assert_eq!(LinkProvider::parse(provider.as_str()), Some(provider));
        }
        assert_eq!(LinkProvider::parse("other"), None);
        let request: LinkCodeRequest = serde_json::from_str(r#"{"provider":"salut"}"#).unwrap();
        assert_eq!(request.provider, LinkProvider::Salut);
    }
}
