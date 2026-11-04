use serde::{Deserialize, Deserializer, Serialize};

use super::Room;
use crate::modules::rooms::listing::image;

pub const FREE_CAPACITY: usize = 10;
pub const FREE_MODS_MAX: usize = 3;
pub const STAR_CAPACITY: usize = 25;
pub const STAR_MODS_MAX: usize = 8;
pub const COVER_ACCENTS: u8 = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Perks {
    pub star_host: bool,
    pub capacity: usize,
    pub mods_max: usize,
}

impl Perks {
    pub fn free() -> Self {
        Self {
            star_host: false,
            capacity: FREE_CAPACITY,
            mods_max: FREE_MODS_MAX,
        }
    }

    pub fn star() -> Self {
        Self {
            star_host: true,
            capacity: STAR_CAPACITY,
            mods_max: STAR_MODS_MAX,
        }
    }

    pub fn of(star_host: bool) -> Self {
        if star_host {
            Self::star()
        } else {
            Self::free()
        }
    }
}

impl Default for Perks {
    fn default() -> Self {
        Self::free()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cover {
    pub url: String,
    pub accent: u8,
}

impl Cover {
    pub fn checked(&self) -> Option<Self> {
        let url = image(&self.url)?;
        (self.accent < COVER_ACCENTS).then_some(Self {
            url,
            accent: self.accent,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RoomImages {
    Members,
    Mods,
    Off,
}

impl RoomImages {
    pub fn resolve(stored: Option<Self>, public: bool) -> Self {
        match (stored, public) {
            (None | Some(Self::Members), true) => Self::Mods,
            (None, false) => Self::Members,
            (Some(policy), _) => policy,
        }
    }
}

impl Room {
    pub fn images_policy(&self) -> RoomImages {
        RoomImages::resolve(self.room_images, self.public)
    }

    pub fn is_full(&self) -> bool {
        self.members.len() >= self.capacity()
    }

    pub fn capacity(&self) -> usize {
        self.perks.capacity
    }

    pub fn cover_shown(&self) -> Option<Cover> {
        self.perks.star_host.then(|| self.cover.clone()).flatten()
    }
}

pub fn nullable<'de, T, D>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    T: Deserialize<'de>,
    D: Deserializer<'de>,
{
    Option::<T>::deserialize(deserializer).map(Some)
}
