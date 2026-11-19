use sc_mobile::{MobileRequest, playlists};
use serde_json::Value;

use crate::handlers::sync_queue::actions::ActionError;
use crate::handlers::sync_queue::model::ClaimedMutation;

const PLAYLIST_PREFIX: &str = "soundcloud:playlists:";
const TRACK_PREFIX: &str = "soundcloud:tracks:";
const SENT_FIELDS: [&str; 3] = ["title", "sharing", "tracks"];

pub(in crate::handlers::sync_queue) fn carries_only_sent_fields(
    mutation: &ClaimedMutation,
) -> bool {
    if mutation.action_type != "playlist_create" {
        return true;
    }
    mutation
        .payload
        .as_ref()
        .and_then(|payload| payload.get("playlist"))
        .and_then(Value::as_object)
        .is_some_and(|playlist| {
            playlist
                .iter()
                .all(|(key, value)| SENT_FIELDS.contains(&key.as_str()) || value.is_null())
        })
}

pub(super) fn create(mutation: &ClaimedMutation) -> Result<MobileRequest, ActionError> {
    let playlist = mutation
        .payload
        .as_ref()
        .and_then(|payload| payload.get("playlist"))
        .ok_or(ActionError::InvalidPayload("playlist"))?;
    let title = playlist
        .get("title")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|title| !title.is_empty())
        .ok_or(ActionError::InvalidPayload("playlist title"))?;
    let public = playlist.get("sharing").and_then(Value::as_str) != Some("private");
    let tracks = track_urns(playlist)?;
    let tracks: Vec<&str> = tracks.iter().map(String::as_str).collect();
    Ok(playlists::create(title, public, &tracks))
}

pub(super) fn delete(target: &str) -> Result<MobileRequest, ActionError> {
    if !target.starts_with(PLAYLIST_PREFIX) {
        return Err(ActionError::InvalidPayload("playlist urn"));
    }
    Ok(playlists::delete(target))
}

pub(super) fn track_urns(playlist: &Value) -> Result<Vec<String>, ActionError> {
    let Some(tracks) = playlist.get("tracks") else {
        return Ok(Vec::new());
    };
    let tracks = tracks
        .as_array()
        .ok_or(ActionError::InvalidPayload("playlist tracks"))?;
    let mut urns: Vec<String> = Vec::with_capacity(tracks.len());
    for track in tracks {
        let raw = match track {
            Value::Object(_) => track
                .get("urn")
                .and_then(scalar)
                .or_else(|| track.get("id").and_then(scalar)),
            track => scalar(track),
        };
        let id = raw
            .as_deref()
            .and_then(|raw| raw.rsplit(':').next())
            .filter(|id| !id.is_empty() && id.bytes().all(|byte| byte.is_ascii_digit()))
            .ok_or(ActionError::InvalidPayload("playlist track"))?;
        let urn = format!("{TRACK_PREFIX}{id}");
        if !urns.contains(&urn) {
            urns.push(urn);
        }
    }
    Ok(urns)
}

fn scalar(value: &Value) -> Option<String> {
    match value {
        Value::String(value) => Some(value.clone()),
        Value::Number(value) => Some(value.to_string()),
        _ => None,
    }
}
