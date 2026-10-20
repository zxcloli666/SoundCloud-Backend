use std::collections::{BTreeSet, HashMap};

use chat_wire::msg::{Class, MessageAad};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::modules::chat::moderation::keys::ChatKeys;
use crate::modules::chat::moderation::text;
use crate::modules::chat::{ChatError, ChatResult, ChatState};

pub const BUNDLE_VERSION: u8 = 1;
pub const UNREADABLE: &str = "[unreadable]";

#[derive(Clone)]
pub struct StoredMessage {
    pub seq: i64,
    pub client_id: Uuid,
    pub sender_user: String,
    pub sender_device: Option<Uuid>,
    pub class: String,
    pub epoch: i32,
    pub target_seq: Option<i64>,
    pub body: Vec<u8>,
    pub created_at: DateTime<Utc>,
}

#[derive(Serialize, Deserialize)]
pub struct Bundle {
    pub v: u8,
    pub conversation_id: Uuid,
    pub seq_from: i64,
    pub seq_to: i64,
    pub messages: Vec<SealedMessage>,
    pub keys: Vec<BundleKey>,
}

#[derive(Serialize, Deserialize)]
pub struct SealedMessage {
    pub seq: i64,
    pub sender: String,
    pub sender_device: Option<Uuid>,
    pub client_id: Uuid,
    pub class: String,
    pub epoch: i32,
    pub target_seq: Option<i64>,
    pub ts: i64,
    #[serde(with = "crate::modules::chat::b64")]
    pub body: Vec<u8>,
}

#[derive(Serialize, Deserialize)]
pub struct BundleKey {
    pub epoch: i32,
    #[serde(with = "crate::modules::chat::b64")]
    pub ck: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Line {
    pub seq: i64,
    pub sender: String,
    pub class: String,
    pub ts: i64,
    pub text: String,
    pub reported: bool,
}

pub struct Capture {
    pub sealed: Vec<u8>,
    pub version: u16,
}

pub async fn capture(
    chat: &ChatState,
    keys: &dyn ChatKeys,
    conversation: Uuid,
    range: (i64, i64),
    messages: &[StoredMessage],
) -> ChatResult<Capture> {
    let epochs: Vec<i32> = messages
        .iter()
        .filter(|message| message.class != Class::System.as_str())
        .map(|message| message.epoch)
        .collect::<BTreeSet<i32>>()
        .into_iter()
        .collect();
    let wrapped = sqlx::query_file!("queries/chat/reports/keys.sql", conversation, &epochs)
        .fetch_all(&chat.pg)
        .await?;
    let mut bundle_keys = Vec::with_capacity(wrapped.len());
    for row in wrapped {
        let version = u16::try_from(row.kek_version).map_err(|_| ChatError::Internal)?;
        let ck = keys.unwrap_ck(version, conversation, row.epoch, &row.wrapped)?;
        bundle_keys.push(BundleKey {
            epoch: row.epoch,
            ck: ck.to_vec(),
        });
    }
    let bundle = Bundle {
        v: BUNDLE_VERSION,
        conversation_id: conversation,
        seq_from: range.0,
        seq_to: range.1,
        messages: messages.iter().map(sealed_message).collect(),
        keys: bundle_keys,
    };
    let plain = serde_json::to_vec(&bundle).map_err(|_| ChatError::Internal)?;
    Ok(Capture {
        sealed: keys.seal_evidence(&plain)?,
        version: keys.reports_current(),
    })
}

fn sealed_message(message: &StoredMessage) -> SealedMessage {
    SealedMessage {
        seq: message.seq,
        sender: message.sender_user.clone(),
        sender_device: message.sender_device,
        client_id: message.client_id,
        class: message.class.clone(),
        epoch: message.epoch,
        target_seq: message.target_seq,
        ts: message.created_at.timestamp_millis(),
        body: message.body.clone(),
    }
}

pub fn open(keys: &dyn ChatKeys, version: u16, sealed: &[u8]) -> ChatResult<Bundle> {
    let plain = keys.open_evidence(version, sealed)?;
    let bundle: Bundle = serde_json::from_slice(&plain).map_err(|_| ChatError::Internal)?;
    if bundle.v != BUNDLE_VERSION {
        return Err(ChatError::Internal);
    }
    Ok(bundle)
}

pub fn lines(bundle: &Bundle) -> Vec<Line> {
    let keys: HashMap<i32, [u8; 32]> = bundle
        .keys
        .iter()
        .filter_map(|key| Some((key.epoch, key.ck.as_slice().try_into().ok()?)))
        .collect();
    bundle
        .messages
        .iter()
        .map(|message| Line {
            seq: message.seq,
            sender: message.sender.clone(),
            class: message.class.clone(),
            ts: message.ts,
            text: readable(bundle.conversation_id, message, &keys),
            reported: (bundle.seq_from..=bundle.seq_to).contains(&message.seq),
        })
        .collect()
}

fn readable(conversation: Uuid, message: &SealedMessage, keys: &HashMap<i32, [u8; 32]>) -> String {
    let Some(class) = Class::parse(&message.class) else {
        return UNREADABLE.to_owned();
    };
    if class == Class::System {
        return text::system(&message.body);
    }
    let Some(ck) = keys.get(&message.epoch) else {
        return UNREADABLE.to_owned();
    };
    let Ok(epoch) = u32::try_from(message.epoch) else {
        return UNREADABLE.to_owned();
    };
    let aad = MessageAad {
        conv: conversation.into_bytes(),
        epoch,
        sender_device: message.sender_device.unwrap_or_default().into_bytes(),
        client_id: message.client_id.into_bytes(),
        class,
    };
    chat_wire::msg::open(ck, &aad, &message.body)
        .map(|envelope| text::render(&envelope))
        .unwrap_or_else(|_| UNREADABLE.to_owned())
}
