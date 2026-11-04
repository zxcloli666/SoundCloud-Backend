use std::sync::Arc;

use serde::Serialize;
use serde_json::Value;

use super::{ONLINE_WINDOW_MS, RoomsService, now_ms};
use crate::error::AppResult;
use crate::modules::rooms::listing::{PublicTrack, image};
use crate::modules::rooms::model::{MAX_MEMBERS, Room};

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PreviewHost {
    pub id: String,
    pub name: String,
    pub avatar_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PreviewTrack {
    pub urn: String,
    pub title: String,
    pub artist: Option<String>,
    pub artwork_url: Option<String>,
    pub duration_ms: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RoomPreview {
    pub code: String,
    pub title: Option<String>,
    pub host: PreviewHost,
    pub track: Option<PreviewTrack>,
    pub listeners: usize,
    pub capacity: usize,
    pub public: bool,
    pub alive: bool,
}

fn duration(track: &Value) -> Option<i64> {
    ["duration", "full_duration", "durationMs"]
        .iter()
        .find_map(|key| track[key].as_i64())
        .filter(|ms| *ms > 0)
}

impl PreviewTrack {
    fn of(room: &Room) -> Option<Self> {
        let urn = room.playback.track_urn.clone()?;
        let track = room.playback.track.as_ref()?;
        let card = PublicTrack::of(track)?;
        Some(Self {
            urn,
            title: card.title,
            artist: card.artist,
            artwork_url: card.artwork_url,
            duration_ms: duration(track),
        })
    }
}

impl RoomPreview {
    pub fn of(room: &Room, online: &[String]) -> Option<Self> {
        let host = room.member(&room.host_id)?;
        let listeners = room
            .members
            .iter()
            .filter(|m| m.user_id == room.host_id || online.contains(&m.user_id))
            .count();
        Some(Self {
            code: room.code.clone(),
            title: room.title.clone(),
            host: PreviewHost {
                id: host.user_id.clone(),
                name: host.name.clone(),
                avatar_url: host.avatar_url.as_deref().and_then(image),
            },
            track: PreviewTrack::of(room),
            listeners,
            capacity: MAX_MEMBERS,
            public: room.public,
            alive: true,
        })
    }
}

impl RoomsService {
    pub async fn preview(&self, code: &str) -> AppResult<Option<Arc<RoomPreview>>> {
        let key = code.to_owned();
        if let Some(cached) = self.previews.get(&key) {
            return Ok(cached);
        }
        let since = now_ms() - ONLINE_WINDOW_MS;
        let preview = self
            .store
            .load_with_presence(std::slice::from_ref(&key), since)
            .await?
            .first()
            .and_then(|(room, online)| RoomPreview::of(room, online))
            .map(Arc::new);
        self.previews.insert(key, preview.clone());
        Ok(preview)
    }
}
