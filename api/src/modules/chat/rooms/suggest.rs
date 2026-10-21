use redis::AsyncCommands;
use serde_json::Value;

use crate::common::sc_ids::{EntityKind, require_ref};
use crate::modules::chat::rooms::lookup::RoomChatMember;
use crate::modules::chat::rooms::system::{self, Authored, SuggestedTrack, SystemRow};
use crate::modules::chat::rooms::{bridge, refusal};
use crate::modules::chat::rt_keys;
use crate::modules::chat::{ChatError, ChatResult, ChatState, sync};
use crate::modules::rooms::listing::{image, text};

pub const MAX_SNAPSHOT_BYTES: usize = 3 * 1024;
pub const COOLDOWN_SECS: u64 = 10;

fn first_text(track: &Value, paths: &[&[&str]]) -> Option<String> {
    paths.iter().find_map(|path| {
        let value = path.iter().fold(track, |value, key| &value[*key]);
        text(value)
    })
}

fn artwork(track: &Value) -> Option<String> {
    ["artworkUrl", "artwork_url"]
        .iter()
        .find_map(|key| image(track[key].as_str()?))
}

fn duration(track: &Value) -> Option<i64> {
    ["durationMs", "duration_ms", "duration"]
        .iter()
        .find_map(|key| track[key].as_i64())
        .filter(|ms| *ms > 0)
}

impl SuggestedTrack {
    pub fn from_snapshot(track: &Value) -> Option<Self> {
        let size = serde_json::to_vec(track).map_or(usize::MAX, |bytes| bytes.len());
        if size > MAX_SNAPSHOT_BYTES || !track.is_object() {
            return None;
        }
        let urn = require_ref(EntityKind::Track, track["urn"].as_str()?)
            .ok()?
            .urn();
        Some(Self {
            urn,
            title: first_text(track, &[&["title"]])?,
            artist: first_text(track, &[&["artist"], &["user", "username"]]),
            artwork_url: artwork(track),
            duration_ms: duration(track),
        })
    }
}

pub async fn cool_down(chat: &ChatState, user_id: &str) -> ChatResult<()> {
    let key = rt_keys::limit("room_suggest", user_id);
    let mut conn = chat.redis.get().await?;
    let taken: Option<String> = redis::cmd("SET")
        .arg(&key)
        .arg(1)
        .arg("NX")
        .arg("EX")
        .arg(COOLDOWN_SECS)
        .query_async(&mut conn)
        .await?;
    if taken.is_some() {
        return Ok(());
    }
    let left: i64 = conn.pttl(&key).await?;
    Err(ChatError::SlowMode {
        retry_ms: u64::try_from(left).unwrap_or(0).max(1),
    })
}

pub async fn suggest_track(
    chat: &ChatState,
    code: &str,
    user_id: &str,
    track: SuggestedTrack,
) -> ChatResult<()> {
    let opened = chat
        .rooms
        .claim_chat(code, user_id)
        .await
        .map_err(refusal)?;
    let member = RoomChatMember::of(&opened.room, user_id, chrono::Utc::now().timestamp_millis())
        .ok_or(ChatError::NotMember)?;
    if let Some(until) = member.muted_until {
        return Err(ChatError::Muted { until });
    }
    cool_down(chat, user_id).await?;
    bridge::ensure_conversation(chat, &opened).await?;
    let name = opened
        .room
        .member(user_id)
        .map_or_else(|| user_id.to_owned(), |m| m.name.clone());
    let rows = [Authored {
        sender: user_id.to_owned(),
        row: SystemRow::Suggest {
            user_id: user_id.to_owned(),
            name,
            track,
        },
    }];
    let Some(seq) = system::post(chat, opened.conversation_id, &rows, false).await? else {
        return Ok(());
    };
    let members: Vec<String> = opened
        .room
        .members
        .iter()
        .map(|m| m.user_id.clone())
        .collect();
    sync::bump::conversation(chat, opened.conversation_id, seq, &members).await
}
