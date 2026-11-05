use axum::http::StatusCode;
use serde::{Deserialize, Serialize};

use super::Room;
use crate::error::{AppError, AppResult};

pub const MAX_BANNED: usize = 200;
pub const MAX_MUTE_SECS: u32 = 24 * 3_600;
const MAX_MUTED: usize = 200;
const MAX_USER_ID_CHARS: usize = 24;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Mute {
    pub user_id: String,
    pub until_ms: i64,
}

fn not_allowed() -> AppError {
    AppError::coded(
        StatusCode::FORBIDDEN,
        "room_not_allowed",
        "Only the host or a moderator can do this",
    )
}

pub(super) fn host_only() -> AppError {
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

fn plausible_user(id: &str) -> bool {
    let digits = id.strip_prefix("lt:").unwrap_or(id);
    id.len() <= MAX_USER_ID_CHARS
        && !digits.is_empty()
        && digits.bytes().all(|b| b.is_ascii_digit())
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
        if !self.is_member(target) && !plausible_user(target) {
            return Err(missing());
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
        if self.moderators.len() >= self.perks.mods_max {
            return Err(AppError::coded(
                StatusCode::CONFLICT,
                "room_moderators_full",
                "This room has all the moderators it can have",
            ));
        }
        self.moderators.push(target.to_owned());
        Ok(())
    }
}

#[cfg(test)]
#[path = "moderation_tests.rs"]
mod tests;
