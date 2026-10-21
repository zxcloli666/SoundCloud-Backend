mod handoff;
mod moderation;
mod playback;
mod title;

use rand::Rng;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::error::{AppError, AppResult};

pub use moderation::{Mute, SettingsPatch};
pub use title::clean_title;

pub const MAX_MEMBERS: usize = 10;
pub const CODE_LEN: usize = 6;
pub const MAX_TRACK_BYTES: usize = 24 * 1024;
pub const MAX_START_LEAD_MS: i64 = 3_000;
const CODE_ALPHABET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
const MAX_NAME_CHARS: usize = 64;
const MAX_AVATAR_CHARS: usize = 512;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Member {
    pub user_id: String,
    pub name: String,
    pub avatar_url: Option<String>,
    pub joined_at: i64,
    pub ready_urn: Option<String>,
    #[serde(default)]
    pub chat_from_seq: i64,
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

fn on() -> bool {
    true
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
    #[serde(default)]
    pub public: bool,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub chat_id: Option<Uuid>,
    #[serde(default)]
    pub moderators: Vec<String>,
    #[serde(default)]
    pub banned: Vec<String>,
    #[serde(default)]
    pub muted: Vec<Mute>,
    #[serde(default)]
    pub slow_mode_s: Option<u32>,
    #[serde(default = "on")]
    pub chat_enabled: bool,
    #[serde(default = "on")]
    pub links: bool,
    #[serde(default)]
    pub unlisted: bool,
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
            chat_from_seq: 0,
        }
    }

    pub fn reading_from(mut self, seq: i64) -> Self {
        self.chat_from_seq = seq.max(0);
        self
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

pub fn not_member() -> AppError {
    AppError::coded(
        axum::http::StatusCode::NOT_FOUND,
        "room_not_member",
        "You are not in this room",
    )
}

pub fn banned() -> AppError {
    AppError::coded(
        axum::http::StatusCode::FORBIDDEN,
        "room_banned",
        "You were banned from this room",
    )
}

impl Room {
    pub fn new(code: String, host: Member, public: bool, now: i64) -> Self {
        Self {
            code,
            host_id: host.user_id.clone(),
            version: 1,
            created_at: now,
            members: vec![host],
            playback: Playback::default(),
            public,
            title: None,
            chat_id: None,
            moderators: Vec::new(),
            banned: Vec::new(),
            muted: Vec::new(),
            slow_mode_s: None,
            chat_enabled: true,
            links: true,
            unlisted: false,
        }
    }

    pub fn is_member(&self, user_id: &str) -> bool {
        self.members.iter().any(|m| m.user_id == user_id)
    }

    pub fn member(&self, user_id: &str) -> Option<&Member> {
        self.members.iter().find(|m| m.user_id == user_id)
    }

    pub fn is_host(&self, user_id: &str) -> bool {
        self.host_id == user_id
    }

    pub fn is_banned(&self, user_id: &str) -> bool {
        self.banned.iter().any(|id| id == user_id)
    }

    pub fn is_full(&self) -> bool {
        self.members.len() >= MAX_MEMBERS
    }

    pub fn set_public(&mut self, user_id: &str, public: bool) -> AppResult<()> {
        if !self.is_host(user_id) {
            return Err(AppError::forbidden(
                "Only the host opens or closes the room",
            ));
        }
        self.public = public;
        Ok(())
    }

    pub fn drop_absent(&mut self, online: &[String], joined_after: i64) {
        let host_id = self.host_id.clone();
        self.members.retain(|m| {
            m.user_id == host_id || m.joined_at > joined_after || online.contains(&m.user_id)
        });
        self.keep_members_only();
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
        if self.is_banned(&member.user_id) {
            return Err(banned());
        }
        if self.is_full() {
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
        self.keep_members_only();
        self.members.len() != before
    }

    fn keep_members_only(&mut self) {
        let members: Vec<String> = self.members.iter().map(|m| m.user_id.clone()).collect();
        self.moderators.retain(|id| members.contains(id));
    }
}

#[cfg(test)]
#[path = "model_tests.rs"]
mod tests;
