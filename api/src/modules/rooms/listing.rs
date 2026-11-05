use serde::Serialize;
use serde_json::Value;

use crate::modules::rooms::model::{
    Backdrop, Cover, FREE_CAPACITY, PlaybackStatus, Room, artwork_url,
};

const MAX_TEXT_CHARS: usize = 120;

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PublicTrack {
    pub title: String,
    pub artist: Option<String>,
    pub artwork_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PublicRoom {
    pub host_id: String,
    pub title: Option<String>,
    pub host_name: String,
    pub host_avatar_url: Option<String>,
    pub status: PlaybackStatus,
    pub track: Option<PublicTrack>,
    pub listeners: usize,
    pub capacity: usize,
    pub full: bool,
    pub star_host: bool,
    pub cover: Option<Cover>,
    pub backdrop: Option<Backdrop>,
    pub created_at: i64,
    #[serde(skip)]
    pub banned: Vec<String>,
}

pub fn text(value: &Value) -> Option<String> {
    let text: String = value
        .as_str()?
        .trim()
        .chars()
        .filter(|c| !c.is_control())
        .take(MAX_TEXT_CHARS)
        .collect();
    (!text.is_empty()).then_some(text)
}

impl PublicTrack {
    pub fn of(track: &Value) -> Option<Self> {
        let artwork = [&track["artwork_url"], &track["user"]["avatar_url"]]
            .into_iter()
            .find_map(|value| artwork_url(value.as_str()?));
        Some(Self {
            title: text(&track["title"])?,
            artist: text(&track["user"]["username"]),
            artwork_url: artwork,
        })
    }
}

impl PublicRoom {
    pub fn of(room: &Room, online: &[String]) -> Option<Self> {
        if room.unlisted {
            return None;
        }
        let host = room.members.iter().find(|m| m.user_id == room.host_id)?;
        let listeners = room
            .members
            .iter()
            .filter(|m| m.user_id == room.host_id || online.contains(&m.user_id))
            .count();
        Some(Self {
            host_id: host.user_id.clone(),
            title: room.title.clone(),
            host_name: host.name.clone(),
            host_avatar_url: host.avatar_url.as_deref().and_then(artwork_url),
            status: room.playback.status,
            track: room.playback.track.as_ref().and_then(PublicTrack::of),
            listeners,
            capacity: room.capacity(),
            full: listeners >= room.capacity(),
            star_host: room.perks.star_host,
            cover: room.cover_shown(),
            backdrop: room.backdrop_in_public(),
            created_at: room.created_at,
            banned: room.banned.clone(),
        })
    }
}

fn ranked_listeners(room: &PublicRoom) -> usize {
    room.listeners.min(FREE_CAPACITY)
}

pub fn rank(rooms: &mut [PublicRoom]) {
    rooms.sort_by(|a, b| {
        a.full
            .cmp(&b.full)
            .then(ranked_listeners(b).cmp(&ranked_listeners(a)))
            .then(a.created_at.cmp(&b.created_at))
            .then(a.host_id.cmp(&b.host_id))
    });
}

#[cfg(test)]
#[path = "listing_tests.rs"]
mod tests;
