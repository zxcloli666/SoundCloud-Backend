use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use serde::{Deserialize, Serialize};

use crate::modules::chat::blocks::{self, BlockedView};
use crate::modules::chat::body::ChatJson;
use crate::modules::chat::sync::prefs::{self, Prefs};
use crate::modules::chat::{ChatCaller, ChatError, ChatResult, ChatState, users};

pub const BLOCKED_SHOWN: i64 = 500;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DmPolicy {
    Everyone,
    Following,
    Nobody,
}

impl DmPolicy {
    pub fn as_str(self) -> &'static str {
        match self {
            DmPolicy::Everyone => "everyone",
            DmPolicy::Following => "following",
            DmPolicy::Nobody => "nobody",
        }
    }

    fn parse(raw: &str) -> Self {
        match raw {
            "everyone" => DmPolicy::Everyone,
            "nobody" => DmPolicy::Nobody,
            _ => DmPolicy::Following,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct UserSettings {
    pub dm_policy: DmPolicy,
    pub read_receipts: bool,
    pub typing: bool,
    pub presence_share: bool,
}

impl Default for UserSettings {
    fn default() -> Self {
        Self {
            dm_policy: DmPolicy::Following,
            read_receipts: true,
            typing: true,
            presence_share: false,
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SettingsPatch {
    pub dm_policy: Option<DmPolicy>,
    pub read_receipts: Option<bool>,
    pub typing: Option<bool>,
    pub presence_share: Option<bool>,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct SettingsView {
    pub dm_policy: DmPolicy,
    pub read_receipts: bool,
    pub typing: bool,
    pub presence_share: bool,
    pub restricted_until: Option<i64>,
    pub restriction_scope: Vec<String>,
    pub blocked: Vec<BlockedView>,
}

pub async fn load(chat: &ChatState, user_id: &str) -> ChatResult<UserSettings> {
    let row = sqlx::query_file!("queries/chat/settings/load.sql", user_id)
        .fetch_optional(&chat.pg)
        .await?;
    Ok(row.map_or_else(UserSettings::default, |row| UserSettings {
        dm_policy: DmPolicy::parse(&row.dm_policy),
        read_receipts: row.read_receipts,
        typing: row.typing,
        presence_share: row.presence_share,
    }))
}

pub async fn save(
    chat: &ChatState,
    user_id: &str,
    patch: &SettingsPatch,
) -> ChatResult<UserSettings> {
    let row = sqlx::query_file!(
        "queries/chat/settings/update.sql",
        user_id,
        patch.dm_policy.map(DmPolicy::as_str),
        patch.read_receipts,
        patch.typing,
        patch.presence_share,
    )
    .fetch_one(&chat.pg)
    .await?;
    Ok(UserSettings {
        dm_policy: DmPolicy::parse(&row.dm_policy),
        read_receipts: row.read_receipts,
        typing: row.typing,
        presence_share: row.presence_share,
    })
}

pub async fn show(
    caller: ChatCaller,
    State(chat): State<Arc<ChatState>>,
) -> ChatResult<Json<SettingsView>> {
    users::touch(&chat, &caller.user_id).await?;
    let settings = load(&chat, &caller.user_id).await?;
    Ok(Json(view(&chat, &caller.user_id, settings).await?))
}

pub async fn update(
    caller: ChatCaller,
    State(chat): State<Arc<ChatState>>,
    ChatJson(patch): ChatJson<SettingsPatch>,
) -> ChatResult<Json<SettingsView>> {
    if patch.dm_policy.is_none()
        && patch.read_receipts.is_none()
        && patch.typing.is_none()
        && patch.presence_share.is_none()
    {
        return Err(ChatError::Invalid);
    }
    let settings = save(&chat, &caller.user_id, &patch).await?;
    let prefs = Prefs {
        read_receipts: settings.read_receipts,
        typing: settings.typing,
        presence_share: settings.presence_share,
    };
    prefs::changed(&chat, &caller.user_id, prefs).await?;
    Ok(Json(view(&chat, &caller.user_id, settings).await?))
}

async fn view(chat: &ChatState, user_id: &str, settings: UserSettings) -> ChatResult<SettingsView> {
    let restriction = users::restriction(chat, user_id).await?;
    let blocked = blocks::list(chat, user_id, BLOCKED_SHOWN).await?;
    Ok(SettingsView {
        dm_policy: settings.dm_policy,
        read_receipts: settings.read_receipts,
        typing: settings.typing,
        presence_share: settings.presence_share,
        restricted_until: restriction
            .as_ref()
            .map(|held| held.until.timestamp_millis()),
        restriction_scope: restriction.map(|held| held.scope).unwrap_or_default(),
        blocked,
    })
}

#[cfg(test)]
#[path = "tests/settings_tests.rs"]
mod tests;
