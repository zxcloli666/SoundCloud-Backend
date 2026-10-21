use chrono::{DateTime, Utc};
use serde::{Deserialize, Deserializer};
use serde_json::{Map, Value};
use sqlx::PgPool;

use crate::modules::chat::conversations::create;
use crate::modules::chat::members::access::Access;
use crate::modules::chat::messages::system::{self, SystemEvent};
use crate::modules::chat::model::MessageRow;
use crate::modules::chat::{ChatError, ChatResult};

pub const MUTE_FOREVER_MS: i64 = 253_402_300_799_000;

#[derive(Debug, Clone, Default, Deserialize)]
pub struct ConversationPatch {
    #[serde(default, with = "crate::modules::chat::b64::option")]
    pub meta_ct: Option<Vec<u8>>,
    pub meta_epoch: Option<i32>,
    pub settings: Option<SettingsPatch>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SettingsPatch {
    pub admins_only: Option<bool>,
    pub history_visible: Option<bool>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct MyPatch {
    #[serde(default, deserialize_with = "present")]
    pub muted_until: Option<Option<i64>>,
    pub notify: Option<Notify>,
    pub pinned: Option<bool>,
    pub archived: Option<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Notify {
    All,
    Mentions,
    None,
}

impl Notify {
    fn as_str(self) -> &'static str {
        match self {
            Notify::All => "all",
            Notify::Mentions => "mentions",
            Notify::None => "none",
        }
    }
}

fn present<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    T::deserialize(deserializer).map(Some)
}

impl SettingsPatch {
    fn merged(&self) -> Map<String, Value> {
        let mut merged = Map::new();
        if let Some(on) = self.admins_only {
            merged.insert("admins_only".to_owned(), Value::Bool(on));
        }
        if let Some(on) = self.history_visible {
            merged.insert("history_visible".to_owned(), Value::Bool(on));
        }
        merged
    }
}

pub async fn apply(
    pg: &PgPool,
    access: &Access,
    patch: ConversationPatch,
) -> ChatResult<Option<MessageRow>> {
    let actor = access.clone().active()?.admin()?;
    let meta = create::meta(patch.meta_ct, patch.meta_epoch)?;
    let settings = patch
        .settings
        .map(|settings| settings.merged())
        .unwrap_or_default();
    if meta.is_none() && settings.is_empty() {
        return Err(ChatError::Invalid);
    }
    let mut tx = pg.begin().await?;
    if !settings.is_empty() {
        sqlx::query_file_scalar!(
            "queries/chat/conversations/merge_settings.sql",
            actor.conversation,
            Value::Object(settings)
        )
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(ChatError::NotFound)?;
    }
    let mut renamed = None;
    if let Some((meta_ct, meta_epoch)) = meta {
        sqlx::query_file_scalar!(
            "queries/chat/conversations/set_meta.sql",
            actor.conversation,
            meta_ct,
            meta_epoch
        )
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(ChatError::NotFound)?;
        let event = SystemEvent::Renamed {
            actor: actor.user_id.clone(),
        };
        renamed = Some(system::insert(&mut tx, actor.conversation, &event, None).await?);
    }
    tx.commit().await?;
    Ok(renamed)
}

pub fn mute_until(raw: Option<i64>) -> ChatResult<Option<DateTime<Utc>>> {
    match raw {
        None => Ok(None),
        Some(ms) if ms > 0 => DateTime::from_timestamp_millis(ms.min(MUTE_FOREVER_MS))
            .map(Some)
            .ok_or(ChatError::Invalid),
        Some(_) => Err(ChatError::Invalid),
    }
}

pub async fn apply_mine(pg: &PgPool, access: &Access, patch: MyPatch) -> ChatResult<()> {
    let (set_mute, muted_until) = match patch.muted_until {
        None => (false, None),
        Some(raw) => (true, mute_until(raw)?),
    };
    sqlx::query_file_scalar!(
        "queries/chat/members/prefs.sql",
        access.conversation,
        access.user_id,
        set_mute,
        muted_until,
        patch.notify.map(Notify::as_str),
        patch.pinned,
        patch.archived
    )
    .fetch_optional(pg)
    .await?
    .ok_or(ChatError::NotMember)?;
    Ok(())
}

#[cfg(test)]
#[path = "../tests/conversation_patch_tests.rs"]
mod tests;
