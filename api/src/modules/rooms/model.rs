use rand::Rng;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::common::sc_ids::{EntityKind, require_ref};
use crate::error::{AppError, AppResult};

pub const MAX_MEMBERS: usize = 10;
pub const CODE_LEN: usize = 6;
pub const MAX_TRACK_BYTES: usize = 24 * 1024;
pub const MAX_START_LEAD_MS: i64 = 3_000;
const CODE_ALPHABET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
const MAX_NAME_CHARS: usize = 64;
const MAX_AVATAR_CHARS: usize = 512;
const MIN_RATE: f64 = 0.5;
const MAX_RATE: f64 = 2.0;
const MAX_CROSSFADE_SEC: u32 = 12;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Member {
    pub user_id: String,
    pub name: String,
    pub avatar_url: Option<String>,
    pub joined_at: i64,
    pub ready_urn: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PlaybackStatus {
    #[default]
    Idle,
    Loading,
    Playing,
    Paused,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Playback {
    pub status: PlaybackStatus,
    pub track: Option<Value>,
    pub track_urn: Option<String>,
    #[serde(default)]
    pub next_track: Option<Value>,
    pub position_ms: i64,
    pub at: i64,
    pub rate: f64,
    #[serde(default)]
    pub crossfade_sec: u32,
}

impl Default for Playback {
    fn default() -> Self {
        Self {
            status: PlaybackStatus::Idle,
            track: None,
            track_urn: None,
            next_track: None,
            position_ms: 0,
            at: 0,
            rate: 1.0,
            crossfade_sec: 0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Room {
    pub code: String,
    pub host_id: String,
    pub version: u64,
    pub created_at: i64,
    pub members: Vec<Member>,
    pub playback: Playback,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Profile {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub avatar_url: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaybackUpdate {
    pub status: PlaybackStatus,
    #[serde(default)]
    pub track: Option<Value>,
    #[serde(default)]
    pub track_urn: Option<String>,
    #[serde(default)]
    pub next_track: Option<Value>,
    #[serde(default)]
    pub position_ms: i64,
    #[serde(default)]
    pub lead_ms: i64,
    #[serde(default)]
    pub rate: Option<f64>,
    #[serde(default)]
    pub crossfade_sec: Option<u32>,
}

pub fn new_code() -> String {
    let mut rng = rand::thread_rng();
    (0..CODE_LEN)
        .map(|_| CODE_ALPHABET[rng.gen_range(0..CODE_ALPHABET.len())] as char)
        .collect()
}

pub fn normalize_code(input: &str) -> Option<String> {
    let code: String = input
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '-')
        .map(|c| c.to_ascii_uppercase())
        .collect();
    let valid = code.len() == CODE_LEN && code.bytes().all(|b| CODE_ALPHABET.contains(&b));
    valid.then_some(code)
}

impl Member {
    pub fn new(user_id: &str, profile: &Profile, now: i64) -> Self {
        Self {
            user_id: user_id.to_owned(),
            name: clean_name(&profile.name, user_id),
            avatar_url: clean_avatar(profile.avatar_url.as_deref()),
            joined_at: now,
            ready_urn: None,
        }
    }
}

fn clean_name(name: &str, fallback: &str) -> String {
    let name: String = name
        .trim()
        .chars()
        .filter(|c| !c.is_control())
        .take(MAX_NAME_CHARS)
        .collect();
    if name.is_empty() {
        fallback.to_owned()
    } else {
        name
    }
}

fn clean_avatar(url: Option<&str>) -> Option<String> {
    let url = url?.trim();
    (url.starts_with("https://") && url.len() <= MAX_AVATAR_CHARS).then(|| url.to_owned())
}

fn check_snapshot(track: &Value) -> AppResult<()> {
    let size = serde_json::to_vec(track)
        .map(|v| v.len())
        .unwrap_or(usize::MAX);
    if size > MAX_TRACK_BYTES || !track.is_object() {
        return Err(AppError::bad_request("track snapshot is too large"));
    }
    Ok(())
}

impl Room {
    pub fn new(code: String, host: Member, now: i64) -> Self {
        Self {
            code,
            host_id: host.user_id.clone(),
            version: 1,
            created_at: now,
            members: vec![host],
            playback: Playback::default(),
        }
    }

    pub fn is_member(&self, user_id: &str) -> bool {
        self.members.iter().any(|m| m.user_id == user_id)
    }

    pub fn is_host(&self, user_id: &str) -> bool {
        self.host_id == user_id
    }

    pub fn join(&mut self, member: Member) -> AppResult<()> {
        if let Some(existing) = self
            .members
            .iter_mut()
            .find(|m| m.user_id == member.user_id)
        {
            existing.name = member.name;
            existing.avatar_url = member.avatar_url;
            return Ok(());
        }
        if self.members.len() >= MAX_MEMBERS {
            return Err(AppError::coded(
                axum::http::StatusCode::CONFLICT,
                "room_full",
                "This listening room is full",
            ));
        }
        self.members.push(member);
        Ok(())
    }

    pub fn remove(&mut self, user_id: &str) -> bool {
        let before = self.members.len();
        self.members.retain(|m| m.user_id != user_id);
        self.members.len() != before
    }

    pub fn set_playback(
        &mut self,
        user_id: &str,
        update: PlaybackUpdate,
        now: i64,
    ) -> AppResult<()> {
        if !self.is_host(user_id) {
            return Err(AppError::forbidden("Only the host controls playback"));
        }
        let track_urn = match update.track_urn.as_deref() {
            Some(urn) => Some(require_ref(EntityKind::Track, urn)?.urn()),
            None => None,
        };
        if track_urn.is_none() && update.status != PlaybackStatus::Idle {
            return Err(AppError::bad_request("trackUrn is required"));
        }
        for snapshot in [&update.track, &update.next_track].into_iter().flatten() {
            check_snapshot(snapshot)?;
        }
        let same_track = track_urn == self.playback.track_urn;
        let track = match update.track {
            Some(track) => Some(track),
            None if same_track => self.playback.track.take(),
            None => None,
        };
        let next_track = match update.next_track {
            Some(next) => Some(next),
            None if same_track => self.playback.next_track.take(),
            None => None,
        };
        self.playback = Playback {
            status: update.status,
            track,
            track_urn,
            next_track,
            position_ms: update.position_ms.max(0),
            at: now + update.lead_ms.clamp(0, MAX_START_LEAD_MS),
            rate: update.rate.unwrap_or(1.0).clamp(MIN_RATE, MAX_RATE),
            crossfade_sec: update
                .crossfade_sec
                .unwrap_or(self.playback.crossfade_sec)
                .min(MAX_CROSSFADE_SEC),
        };
        Ok(())
    }

    pub fn mark_ready(&mut self, user_id: &str, track_urn: &str) -> AppResult<bool> {
        let urn = require_ref(EntityKind::Track, track_urn)?.urn();
        let member = self
            .members
            .iter_mut()
            .find(|m| m.user_id == user_id)
            .ok_or_else(|| AppError::not_found("You are not in this room"))?;
        if member.ready_urn.as_deref() == Some(urn.as_str()) {
            return Ok(false);
        }
        member.ready_urn = Some(urn);
        Ok(true)
    }
}

#[cfg(test)]
#[path = "model_tests.rs"]
mod tests;
