use axum::http::StatusCode;
use serde::{Deserialize, Serialize};

use super::{Room, clean_title};
use crate::error::{AppError, AppResult};

pub const MAX_MODERATORS: usize = 3;
pub const MAX_BANNED: usize = 200;
pub const MAX_MUTE_SECS: u32 = 24 * 3_600;
pub const MAX_SLOW_MODE_SECS: u32 = 600;
pub const PUBLIC_SLOW_MODE_SECS: u32 = 3;
const MAX_MUTED: usize = 200;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Mute {
    pub user_id: String,
    pub until_ms: i64,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct SettingsPatch {
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default, alias = "slowModeS")]
    pub slow_mode_s: Option<u32>,
    #[serde(default, alias = "chatEnabled")]
    pub chat_enabled: Option<bool>,
    #[serde(default)]
    pub links: Option<bool>,
}

fn not_allowed() -> AppError {
    AppError::coded(
        StatusCode::FORBIDDEN,
        "room_not_allowed",
        "Only the host or a moderator can do this",
    )
}

fn host_only() -> AppError {
    AppError::coded(
        StatusCode::FORBIDDEN,
        "room_host_only",
        "Only the host can do this",
    )
}

fn missing() -> AppError {
    AppError::coded(
        StatusCode::NOT_FOUND,
        "room_member_missing",
        "This person is not in the room",
    )
}

fn trim_front<T>(list: &mut Vec<T>, max: usize) {
    let overflow = list.len().saturating_sub(max);
    list.drain(..overflow);
}

impl Room {
    pub fn is_moderator(&self, user_id: &str) -> bool {
        self.moderators.iter().any(|id| id == user_id)
    }

    pub fn can_moderate(&self, user_id: &str) -> bool {
        self.is_host(user_id) || self.is_moderator(user_id)
    }

    fn may_act_on(&self, actor: &str, target: &str) -> AppResult<()> {
        let outranked = self.is_host(target) || (!self.is_host(actor) && self.is_moderator(target));
        if !self.can_moderate(actor) || actor == target || outranked {
            return Err(not_allowed());
        }
        Ok(())
    }

    pub fn kick(&mut self, actor: &str, target: &str) -> AppResult<()> {
        self.may_act_on(actor, target)?;
        if !self.remove(target) {
            return Err(missing());
        }
        Ok(())
    }

    pub fn ban(&mut self, actor: &str, target: &str) -> AppResult<()> {
        if !self.is_host(actor) {
            return Err(host_only());
        }
        if actor == target {
            return Err(not_allowed());
        }
        self.remove(target);
        if !self.is_banned(target) {
            self.banned.push(target.to_owned());
        }
        trim_front(&mut self.banned, MAX_BANNED);
        Ok(())
    }

    pub fn mute(&mut self, actor: &str, target: &str, seconds: u32, now: i64) -> AppResult<()> {
        self.may_act_on(actor, target)?;
        if !self.is_member(target) {
            return Err(missing());
        }
        self.muted
            .retain(|m| m.until_ms > now && m.user_id != target);
        if seconds > 0 {
            self.muted.push(Mute {
                user_id: target.to_owned(),
                until_ms: now + i64::from(seconds.min(MAX_MUTE_SECS)) * 1_000,
            });
        }
        trim_front(&mut self.muted, MAX_MUTED);
        Ok(())
    }

    pub fn muted_until(&self, user_id: &str, now: i64) -> Option<i64> {
        self.muted
            .iter()
            .find(|m| m.user_id == user_id && m.until_ms > now)
            .map(|m| m.until_ms)
    }

    pub fn set_moderator(&mut self, actor: &str, target: &str, on: bool) -> AppResult<()> {
        if !self.is_host(actor) {
            return Err(host_only());
        }
        if self.is_host(target) {
            return Err(not_allowed());
        }
        if !on {
            self.moderators.retain(|id| id != target);
            return Ok(());
        }
        if !self.is_member(target) {
            return Err(missing());
        }
        if self.is_moderator(target) {
            return Ok(());
        }
        if self.moderators.len() >= MAX_MODERATORS {
            return Err(AppError::coded(
                StatusCode::CONFLICT,
                "room_moderators_full",
                "A room has at most three moderators",
            ));
        }
        self.moderators.push(target.to_owned());
        Ok(())
    }

    pub fn apply_settings(&mut self, actor: &str, patch: &SettingsPatch) -> AppResult<()> {
        if !self.is_host(actor) {
            return Err(host_only());
        }
        if let Some(title) = &patch.title {
            self.title = clean_title(title);
        }
        if let Some(seconds) = patch.slow_mode_s {
            self.slow_mode_s = Some(seconds.min(MAX_SLOW_MODE_SECS));
        }
        if let Some(on) = patch.chat_enabled {
            self.chat_enabled = on;
        }
        if let Some(on) = patch.links {
            self.links = on;
        }
        Ok(())
    }

    pub fn slow_mode(&self) -> u32 {
        let default = if self.public {
            PUBLIC_SLOW_MODE_SECS
        } else {
            0
        };
        self.slow_mode_s.unwrap_or(default)
    }
}

#[cfg(test)]
#[path = "moderation_tests.rs"]
mod tests;
