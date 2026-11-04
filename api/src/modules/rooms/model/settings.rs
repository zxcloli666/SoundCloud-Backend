use axum::http::StatusCode;
use serde::Deserialize;

use super::moderation::host_only;
use super::perks::nullable;
use super::{Backdrop, Cover, Room, RoomImages, clean_title};
use crate::error::{AppError, AppResult};

pub const MAX_SLOW_MODE_SECS: u32 = 600;
pub const PUBLIC_SLOW_MODE_SECS: u32 = 3;
pub const BACKDROP_COOLDOWN: &str = "room_backdrop_cooldown";

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
    #[serde(default, deserialize_with = "nullable")]
    pub cover: Option<Option<Cover>>,
    #[serde(default, alias = "roomImages")]
    pub room_images: Option<RoomImages>,
    #[serde(default, deserialize_with = "nullable")]
    pub backdrop: Option<Option<Backdrop>>,
}

impl SettingsPatch {
    pub fn wanted_image(&self) -> Option<uuid::Uuid> {
        self.backdrop.as_ref()?.as_ref()?.image()
    }

    fn sets_star_perk(&self) -> bool {
        matches!(self.cover, Some(Some(_))) || matches!(self.backdrop, Some(Some(_)))
    }
}

fn star_required() -> AppError {
    AppError::coded(
        StatusCode::FORBIDDEN,
        "room_star_required",
        "Only a STAR host can do this",
    )
}

fn invalid() -> AppError {
    AppError::coded(
        StatusCode::BAD_REQUEST,
        "room_invalid",
        "This room setting is not valid",
    )
}

fn cooling_down() -> AppError {
    AppError::coded(
        StatusCode::FORBIDDEN,
        BACKDROP_COOLDOWN,
        "The room image backdrop is paused for now",
    )
}

impl Room {
    pub fn apply_settings(
        &mut self,
        actor: &str,
        patch: &SettingsPatch,
        now: i64,
    ) -> AppResult<()> {
        if !self.is_host(actor) {
            return Err(host_only());
        }
        if patch.sets_star_perk() && !self.perks.star_host {
            return Err(star_required());
        }
        if let Some(cover) = &patch.cover {
            self.cover = cover
                .as_ref()
                .map(|cover| cover.checked().ok_or_else(invalid))
                .transpose()?;
        }
        if let Some(policy) = patch.room_images {
            if policy == RoomImages::Members && self.public {
                return Err(invalid());
            }
            self.room_images = Some(policy);
        }
        if let Some(backdrop) = &patch.backdrop {
            self.set_backdrop(backdrop.as_ref(), now)?;
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

    fn set_backdrop(&mut self, backdrop: Option<&Backdrop>, now: i64) -> AppResult<()> {
        let Some(backdrop) = backdrop else {
            self.backdrop = None;
            return Ok(());
        };
        if !backdrop.known() {
            return Err(invalid());
        }
        if backdrop.image().is_some() && self.backdrop_cooldown(now).is_some() {
            return Err(cooling_down());
        }
        self.backdrop = Some(backdrop.clone());
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
#[path = "settings_tests.rs"]
mod tests;
