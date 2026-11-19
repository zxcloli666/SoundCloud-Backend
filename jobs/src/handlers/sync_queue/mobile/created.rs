use serde_json::{Map, Value, json};

use crate::handlers::sync_queue::actions::{ActionError, Remote};
use crate::handlers::sync_queue::model::ClaimedMutation;

pub(in crate::handlers::sync_queue) fn shape(
    mutation: &ClaimedMutation,
    remote: Remote,
) -> Result<Remote, ActionError> {
    match remote {
        Remote::Written(reply) if mutation.action_type == "playlist_create" => {
            created_playlist(mutation, &reply).map(Remote::Written)
        }
        remote => Ok(remote),
    }
}

fn created_playlist(mutation: &ClaimedMutation, reply: &Value) -> Result<Value, ActionError> {
    let sent = mutation
        .payload
        .as_ref()
        .and_then(|payload| payload.get("playlist"));
    let mut playlist: Map<String, Value> = reply
        .get("playlist")
        .unwrap_or(reply)
        .as_object()
        .cloned()
        .ok_or(ActionError::InvalidRemoteResult(
            "mobile playlist reply is not an object",
        ))?;
    let urn = urn_of(&playlist).ok_or(ActionError::InvalidRemoteResult(
        "mobile playlist reply names no playlist",
    ))?;
    playlist.insert("urn".to_owned(), json!(urn));
    if !playlist.contains_key("title") {
        playlist.insert(
            "title".to_owned(),
            sent.and_then(|sent| sent.get("title"))
                .cloned()
                .unwrap_or(Value::Null),
        );
    }
    if !playlist.get("sharing").is_some_and(Value::is_string) {
        let sharing = match playlist.get("public").and_then(Value::as_bool) {
            Some(false) => "private",
            Some(true) => "public",
            None => sent
                .and_then(|sent| sent.get("sharing"))
                .and_then(Value::as_str)
                .unwrap_or("public"),
        };
        playlist.insert("sharing".to_owned(), json!(sharing));
    }
    if !playlist.contains_key("user") {
        let id = mutation
            .user_id
            .rsplit(':')
            .next()
            .unwrap_or(&mutation.user_id);
        playlist.insert(
            "user".to_owned(),
            json!({"id": id, "urn": format!("soundcloud:users:{id}")}),
        );
    }
    if !playlist.contains_key("tracks") {
        let tracks = match playlist.get("track_urns").and_then(Value::as_array) {
            Some(urns) => urns.iter().map(|urn| json!({"urn": urn})).collect(),
            None => match sent {
                Some(sent) => super::playlist::track_urns(sent)?
                    .into_iter()
                    .map(|urn| json!({"urn": urn}))
                    .collect(),
                None => Vec::new(),
            },
        };
        playlist.insert("tracks".to_owned(), Value::Array(tracks));
    }
    Ok(Value::Object(playlist))
}

fn urn_of(playlist: &Map<String, Value>) -> Option<String> {
    if let Some(urn) = playlist
        .get("urn")
        .and_then(Value::as_str)
        .filter(|urn| urn.starts_with("soundcloud:playlists:"))
    {
        return Some(urn.to_owned());
    }
    playlist
        .get("id")
        .and_then(Value::as_u64)
        .map(|id| format!("soundcloud:playlists:{id}"))
}
