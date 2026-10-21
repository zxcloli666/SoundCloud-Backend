pub mod bind;
mod cache;
mod register;
mod rules;
mod store;

use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use chrono::{DateTime, Duration, Utc};
use serde::Deserialize;
use uuid::Uuid;

use crate::modules::chat::body::{ChatJson, ChatPath};
use crate::modules::chat::caller::CallerDevice;
use crate::modules::chat::history_gate::is_unlocked;
use crate::modules::chat::model::{DeviceRow, DeviceStatus, DeviceView};
use crate::modules::chat::{ChatCaller, ChatError, ChatResult, ChatState};

pub use cache::DeviceCache;

const TOUCH_EVERY_MINUTES: i64 = 10;

#[derive(Debug, Clone, Deserialize)]
pub struct RegisterDevice {
    pub platform: String,
    pub name: String,
    pub app_version: String,
    #[serde(default)]
    pub caps: Vec<String>,
}

pub async fn register(
    caller: ChatCaller,
    State(chat): State<Arc<ChatState>>,
    ChatJson(request): ChatJson<RegisterDevice>,
) -> ChatResult<Json<DeviceView>> {
    let row = register::register(&chat, &caller, request).await?;
    Ok(Json(view(&row, Some(row.id), Utc::now())))
}

pub async fn list(
    caller: ChatCaller,
    State(chat): State<Arc<ChatState>>,
) -> ChatResult<Json<Vec<DeviceView>>> {
    let me = require(&chat, &caller).await?;
    let now = Utc::now();
    let rows = live_of(&chat, &caller.user_id).await?;
    Ok(Json(
        rows.iter().map(|row| view(row, Some(me.id), now)).collect(),
    ))
}

pub async fn remove(
    caller: ChatCaller,
    State(chat): State<Arc<ChatState>>,
    ChatPath(id): ChatPath<Uuid>,
) -> ChatResult<StatusCode> {
    let me = require(&chat, &caller).await?;
    if id == me.id {
        revoke(&chat, &me).await?;
        return Ok(StatusCode::NO_CONTENT);
    }
    let target = owned(&chat, &caller.user_id, id).await?;
    if !rules::may_remove(&me, &target, Utc::now()) {
        return Err(ChatError::TooYoung);
    }
    revoke(&chat, &target).await?;
    sign_out(&chat, &target).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn remove_me(
    caller: ChatCaller,
    State(chat): State<Arc<ChatState>>,
) -> ChatResult<StatusCode> {
    let Some(device) = &caller.device else {
        return Ok(StatusCode::NO_CONTENT);
    };
    let mine = lookup(&chat, device.id)
        .await?
        .filter(|row| row.user_id == caller.user_id);
    if let Some(row) = mine {
        revoke(&chat, &row).await?;
    }
    Ok(StatusCode::NO_CONTENT)
}

pub async fn confirm(
    caller: ChatCaller,
    State(chat): State<Arc<ChatState>>,
    ChatPath(id): ChatPath<Uuid>,
) -> ChatResult<StatusCode> {
    let me = require(&chat, &caller).await?;
    let target = owned(&chat, &caller.user_id, id).await?;
    if target.status != "active" {
        return Err(ChatError::NotFound);
    }
    if !rules::may_confirm(&me, &target, Utc::now()) {
        return Err(ChatError::TooYoung);
    }
    store::unlock_history(&chat.pg, target.id).await?;
    changed(&chat, &target.user_id, &[target.id]);
    Ok(StatusCode::NO_CONTENT)
}

pub async fn not_me(
    caller: ChatCaller,
    State(chat): State<Arc<ChatState>>,
    ChatPath(id): ChatPath<Uuid>,
) -> ChatResult<StatusCode> {
    let me = require(&chat, &caller).await?;
    let target = owned(&chat, &caller.user_id, id).await?;
    if !rules::may_disown(&me, &target) {
        return Err(ChatError::TooYoung);
    }
    revoke(&chat, &target).await?;
    sign_out(&chat, &target).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn lookup(chat: &ChatState, id: Uuid) -> ChatResult<Option<DeviceRow>> {
    if let Some(row) = chat.devices.row(id) {
        return Ok(Some(row));
    }
    let stamp = chat.devices.stamp();
    let row = store::by_id(&chat.pg, id).await?;
    if let Some(row) = &row {
        chat.devices.keep_row(stamp, row.clone());
    }
    Ok(row)
}

pub async fn live_of(chat: &ChatState, user_id: &str) -> ChatResult<Arc<Vec<DeviceRow>>> {
    if let Some(rows) = chat.devices.user(user_id) {
        return Ok(rows);
    }
    let stamp = chat.devices.stamp();
    let rows = Arc::new(store::live_by_user(&chat.pg, user_id).await?);
    chat.devices.keep_user(stamp, user_id, rows.clone());
    Ok(rows)
}

pub async fn alerts(
    chat: &ChatState,
    user_id: &str,
    viewer: &CallerDevice,
) -> ChatResult<Vec<DeviceView>> {
    let now = Utc::now();
    let rows = live_of(chat, user_id).await?;
    Ok(rows
        .iter()
        .filter(|row| row.created_at > viewer.created_at)
        .filter(|row| !is_unlocked(row.created_at, row.history_unlocked_at.is_some(), now))
        .map(|row| view(row, Some(viewer.id), now))
        .collect())
}

pub async fn require(chat: &ChatState, caller: &ChatCaller) -> ChatResult<DeviceRow> {
    let device = caller.device.as_ref().ok_or(ChatError::DeviceRevoked)?;
    let row = lookup(chat, device.id)
        .await?
        .ok_or(ChatError::DeviceRevoked)?;
    if row.user_id != caller.user_id || row.status != "active" {
        return Err(ChatError::DeviceRevoked);
    }
    Ok(row)
}

pub async fn revoke(chat: &ChatState, row: &DeviceRow) -> ChatResult<bool> {
    let mut tx = chat.pg.begin().await?;
    let revoked = store::revoke(&mut *tx, row.id).await?;
    if revoked {
        store::rekey_user(&mut *tx, &row.user_id).await?;
    }
    tx.commit().await?;
    changed(chat, &row.user_id, &[row.id]);
    Ok(revoked)
}

pub async fn touch(chat: &ChatState, row: &DeviceRow) -> ChatResult<()> {
    if Utc::now() - row.last_seen_at < Duration::minutes(TOUCH_EVERY_MINUTES) {
        return Ok(());
    }
    store::touch(&chat.pg, row.id).await?;
    chat.devices.forget(&row.user_id, &[row.id]);
    Ok(())
}

pub fn view(row: &DeviceRow, current: Option<Uuid>, now: DateTime<Utc>) -> DeviceView {
    DeviceView {
        id: row.id,
        platform: row.platform.clone(),
        name: row.name.clone(),
        app_version: row.app_version.clone(),
        tier: row.tier.clone(),
        status: if row.status == "active" {
            DeviceStatus::Active
        } else {
            DeviceStatus::Revoked
        },
        created_at: row.created_at.timestamp_millis(),
        last_seen_at: row.last_seen_at.timestamp_millis(),
        history_unlocked: is_unlocked(row.created_at, row.history_unlocked_at.is_some(), now),
        current: current == Some(row.id),
    }
}

impl From<&DeviceRow> for CallerDevice {
    fn from(row: &DeviceRow) -> Self {
        Self {
            id: row.id,
            created_at: row.created_at,
            history_unlocked: row.history_unlocked_at.is_some(),
        }
    }
}

async fn owned(chat: &ChatState, user_id: &str, id: Uuid) -> ChatResult<DeviceRow> {
    lookup(chat, id)
        .await?
        .filter(|row| row.user_id == user_id)
        .ok_or(ChatError::NotFound)
}

async fn sign_out(chat: &ChatState, target: &DeviceRow) -> ChatResult<()> {
    if store::session_held(&chat.pg, target.session_id).await? {
        return Ok(());
    }
    chat.auth.logout(target.session_id).await.map_err(|error| {
        tracing::error!(%error, "chat could not end the session of a removed device");
        ChatError::Internal
    })
}

fn changed(chat: &ChatState, user_id: &str, ids: &[Uuid]) {
    chat.devices.forget(user_id, ids);
    chat.hub.wake(user_id);
}

#[cfg(test)]
#[path = "tests/device_test_support.rs"]
pub mod test_support;

#[cfg(test)]
#[path = "tests/devices_tests.rs"]
mod tests;
