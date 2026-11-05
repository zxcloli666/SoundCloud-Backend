use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::Room;

pub const PRESETS: [&str; 13] = [
    "aurora",
    "vinyl",
    "sunset-clouds",
    "ocean-rays",
    "dunes",
    "misty-forest",
    "snow-lamp",
    "satin",
    "paper",
    "peach-clouds",
    "pastel-lake",
    "watercolor",
    "candy-sky",
];
pub const IMAGE_COOLDOWN_MS: i64 = 60 * 60 * 1_000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Backdrop {
    Preset {
        id: String,
    },
    Image {
        #[serde(rename = "mediaId", alias = "media_id")]
        media_id: Uuid,
    },
}

impl Backdrop {
    pub fn known(&self) -> bool {
        match self {
            Backdrop::Preset { id } => PRESETS.contains(&id.as_str()),
            Backdrop::Image { .. } => true,
        }
    }

    pub fn image(&self) -> Option<Uuid> {
        match self {
            Backdrop::Image { media_id } => Some(*media_id),
            Backdrop::Preset { .. } => None,
        }
    }
}

impl Room {
    pub fn backdrop_shown(&self) -> Option<Backdrop> {
        self.perks
            .star_host
            .then(|| self.backdrop.clone())
            .flatten()
    }

    pub fn backdrop_in_public(&self) -> Option<Backdrop> {
        self.backdrop_shown()
            .filter(|backdrop| backdrop.image().is_none())
    }

    pub fn image_backdrop(&self) -> Option<Uuid> {
        self.backdrop.as_ref().and_then(Backdrop::image)
    }

    pub fn image_shown(&self) -> Option<Uuid> {
        self.backdrop_shown().as_ref().and_then(Backdrop::image)
    }

    pub fn drop_image_backdrop(&mut self) {
        if self.image_backdrop().is_some() {
            self.backdrop = None;
        }
    }

    pub fn clear_image_backdrop(&mut self, media: Uuid) -> bool {
        if self.image_backdrop() != Some(media) {
            return false;
        }
        self.backdrop = None;
        true
    }
}
