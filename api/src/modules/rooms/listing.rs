use serde::Serialize;
use serde_json::Value;

use crate::modules::rooms::model::{MAX_MEMBERS, PlaybackStatus, Room};

const MAX_TEXT_CHARS: usize = 120;
const MAX_IMAGE_CHARS: usize = 512;
const IMAGE_HOST: &str = "sndcdn.com";

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
    pub host_name: String,
    pub host_avatar_url: Option<String>,
    pub status: PlaybackStatus,
    pub track: Option<PublicTrack>,
    pub listeners: usize,
    pub capacity: usize,
    pub full: bool,
    pub created_at: i64,
}

fn text(value: &Value) -> Option<String> {
    let text: String = value
        .as_str()?
        .trim()
        .chars()
        .filter(|c| !c.is_control())
        .take(MAX_TEXT_CHARS)
        .collect();
    (!text.is_empty()).then_some(text)
}

fn image(raw: &str) -> Option<String> {
    if raw.len() > MAX_IMAGE_CHARS {
        return None;
    }
    let url = url::Url::parse(raw).ok()?;
    let host = url.host_str()?;
    let trusted = host == IMAGE_HOST
        || host
            .strip_suffix(IMAGE_HOST)
            .is_some_and(|rest| rest.ends_with('.'));
    (url.scheme() == "https" && trusted).then(|| url.to_string())
}

impl PublicTrack {
    fn of(track: &Value) -> Option<Self> {
        let artwork = [&track["artwork_url"], &track["user"]["avatar_url"]]
            .into_iter()
            .find_map(|value| image(value.as_str()?));
        Some(Self {
            title: text(&track["title"])?,
            artist: text(&track["user"]["username"]),
            artwork_url: artwork,
        })
    }
}

impl PublicRoom {
    pub fn of(room: &Room, online: &[String]) -> Option<Self> {
        let host = room.members.iter().find(|m| m.user_id == room.host_id)?;
        let listeners = room
            .members
            .iter()
            .filter(|m| m.user_id == room.host_id || online.contains(&m.user_id))
            .count();
        Some(Self {
            host_id: host.user_id.clone(),
            host_name: host.name.clone(),
            host_avatar_url: host.avatar_url.as_deref().and_then(image),
            status: room.playback.status,
            track: room.playback.track.as_ref().and_then(PublicTrack::of),
            listeners,
            capacity: MAX_MEMBERS,
            full: listeners >= MAX_MEMBERS,
            created_at: room.created_at,
        })
    }
}

pub fn rank(rooms: &mut [PublicRoom]) {
    rooms.sort_by(|a, b| {
        a.full
            .cmp(&b.full)
            .then(b.listeners.cmp(&a.listeners))
            .then(a.created_at.cmp(&b.created_at))
            .then(a.host_id.cmp(&b.host_id))
    });
}

#[cfg(test)]
#[path = "listing_tests.rs"]
mod tests;
