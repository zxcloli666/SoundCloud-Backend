mod access;
pub mod rotate;

use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::modules::chat::body::{ChatPath, ChatQuery};
use crate::modules::chat::caller::CallerDevice;
use crate::modules::chat::kek::wire_epoch;
use crate::modules::chat::limits::{self, Bucket};
use crate::modules::chat::model::DeviceRow;
use crate::modules::chat::{ChatCaller, ChatError, ChatResult, ChatState, devices, history_gate};

pub const PAGE_MAX: u32 = 64;
const SEALED_KEY_BYTES: u32 = 160;

#[derive(Debug, Clone, Deserialize)]
pub struct KeysQuery {
    pub from_epoch: Option<i32>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SealedKey {
    pub epoch: i32,
    #[serde(with = "crate::modules::chat::b64")]
    pub sealed: Vec<u8>,
}

struct StoredKey {
    epoch: i32,
    wrapped: Vec<u8>,
    kek_version: i16,
}

pub async fn list(
    caller: ChatCaller,
    State(chat): State<Arc<ChatState>>,
    ChatPath(conversation): ChatPath<Uuid>,
    ChatQuery(query): ChatQuery<KeysQuery>,
) -> ChatResult<Json<Vec<SealedKey>>> {
    let device = devices::require(&chat, &caller).await?;
    limits::admit(&chat, Bucket::ReadKeys, &caller.user_id).await?;
    let access = access::access(&chat, conversation, &caller.user_id).await?;
    if access.needs_key {
        rotate::ensure_current(&chat, conversation).await?;
    }
    let gate = history_gate::first_visible_epoch(&chat, &CallerDevice::from(&device), conversation)
        .await?;
    let floor = query
        .from_epoch
        .unwrap_or(0)
        .max(access.member_floor)
        .max(gate);
    let stored = sqlx::query_file_as!(
        StoredKey,
        "queries/chat/keys/list.sql",
        conversation,
        floor,
        i64::from(page_size(caller.max_bytes))
    )
    .fetch_all(&chat.pg)
    .await?;
    let sealed = stored
        .iter()
        .map(|key| seal_for(&chat, &device, conversation, key))
        .collect::<ChatResult<Vec<_>>>()?;
    Ok(Json(sealed))
}

pub async fn conversation_key(
    chat: &ChatState,
    conversation: Uuid,
    epoch: i32,
) -> ChatResult<[u8; 32]> {
    let stored = sqlx::query_file_as!(
        StoredKey,
        "queries/chat/keys/by_epoch.sql",
        conversation,
        epoch
    )
    .fetch_optional(&chat.pg)
    .await?
    .ok_or(ChatError::NotFound)?;
    unwrap(chat, conversation, &stored)
}

fn seal_for(
    chat: &ChatState,
    device: &DeviceRow,
    conversation: Uuid,
    stored: &StoredKey,
) -> ChatResult<SealedKey> {
    let ck = unwrap(chat, conversation, stored)?;
    let ikm_e: [u8; 32] = chat_wire::random_bytes().map_err(|_| ChatError::Internal)?;
    let sealed = chat_wire::ck::seal_for_device(
        &device.box_pub,
        conversation.as_bytes(),
        wire_epoch(stored.epoch)?,
        device.id.as_bytes(),
        &ck,
        ikm_e,
    )
    .map_err(|_| ChatError::Internal)?;
    Ok(SealedKey {
        epoch: stored.epoch,
        sealed,
    })
}

fn unwrap(chat: &ChatState, conversation: Uuid, stored: &StoredKey) -> ChatResult<[u8; 32]> {
    let kek_version = u16::try_from(stored.kek_version).map_err(|_| ChatError::Internal)?;
    chat.keks
        .unwrap(kek_version, conversation, stored.epoch, &stored.wrapped)
}

fn page_size(max_bytes: u32) -> u32 {
    (max_bytes / SEALED_KEY_BYTES).clamp(1, PAGE_MAX)
}

#[cfg(test)]
#[path = "tests/keys_tests.rs"]
mod tests;
