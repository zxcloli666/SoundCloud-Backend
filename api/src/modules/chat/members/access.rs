use serde::Deserialize;
use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;

use crate::modules::chat::conversations::keys::peer_of;
use crate::modules::chat::members::roles;
use crate::modules::chat::model::{ConversationKind, MemberRole, MemberState};
use crate::modules::chat::{ChatError, ChatResult};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
pub struct GroupSettings {
    #[serde(default)]
    pub admins_only: bool,
    #[serde(default = "visible_by_default")]
    pub history_visible: bool,
}

fn visible_by_default() -> bool {
    true
}

impl Default for GroupSettings {
    fn default() -> Self {
        Self {
            admins_only: false,
            history_visible: true,
        }
    }
}

impl GroupSettings {
    pub fn of(raw: &Value) -> Self {
        serde_json::from_value(raw.clone()).unwrap_or_default()
    }
}

#[derive(Clone, Debug)]
pub struct Access {
    pub conversation: Uuid,
    pub user_id: String,
    pub kind: ConversationKind,
    pub peer: Option<String>,
    pub key_epoch: i32,
    pub last_seq: i64,
    pub ttl_s: Option<i32>,
    pub settings: GroupSettings,
    pub role: MemberRole,
    pub state: MemberState,
    pub accepted: bool,
    pub joined_seq: i64,
    pub hidden_before_seq: i64,
    pub peer_state: Option<MemberState>,
    pub recipients: Vec<String>,
}

impl Access {
    pub fn is_admin(&self) -> bool {
        roles::is_admin(self.role)
    }

    pub fn is_group(&self) -> bool {
        self.kind == ConversationKind::Group
    }

    pub fn active(self) -> ChatResult<Self> {
        match self.state {
            MemberState::Active => Ok(self),
            _ => Err(ChatError::NotMember),
        }
    }

    pub fn readable(self) -> ChatResult<Self> {
        match self.state {
            MemberState::Active | MemberState::Invited => Ok(self),
            _ => Err(ChatError::NotMember),
        }
    }

    pub fn admin(self) -> ChatResult<Self> {
        if self.is_group() && self.is_admin() {
            Ok(self)
        } else {
            Err(ChatError::NotMember)
        }
    }

    pub fn history_floor(&self) -> i64 {
        let joined = if self.is_group() && !self.settings.history_visible {
            self.joined_seq
        } else {
            0
        };
        joined.max(self.hidden_before_seq)
    }

    pub fn recipients_with(&self, extra: &str) -> Vec<String> {
        let mut users = self.recipients.clone();
        if !users.iter().any(|user| user == extra) {
            users.push(extra.to_owned());
        }
        users
    }
}

pub async fn load(pg: &PgPool, conversation: Uuid, user_id: &str) -> ChatResult<Access> {
    let row = sqlx::query_file!("queries/chat/members/access.sql", conversation, user_id)
        .fetch_optional(pg)
        .await?
        .ok_or(ChatError::NotMember)?;
    let kind = roles::kind(&row.kind)?;
    Ok(Access {
        conversation,
        user_id: user_id.to_owned(),
        peer: match kind {
            ConversationKind::Direct => row.dm_key.as_deref().and_then(|key| peer_of(key, user_id)),
            _ => None,
        },
        kind,
        key_epoch: row.key_epoch,
        last_seq: row.last_seq,
        ttl_s: row.ttl_s,
        settings: GroupSettings::of(&row.settings),
        role: roles::role(&row.role)?,
        state: roles::state(&row.state)?,
        accepted: row.accepted,
        joined_seq: row.joined_seq,
        hidden_before_seq: row.hidden_before_seq,
        peer_state: row.peer_state.as_deref().map(roles::state).transpose()?,
        recipients: row.recipients,
    })
}
