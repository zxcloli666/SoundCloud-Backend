use sc_mobile::{Caller, MobileClient, playlists};
use serde_json::{Value, json};

use crate::actions::{LEFT, step};
use crate::datadome;
use crate::journal::{Journal, json_of};

const TITLE: &str = "probe";

pub async fn run(
    client: &MobileClient,
    caller: &Caller,
    journal: &mut Journal,
    track: Option<&str>,
) {
    let create = playlists::create(TITLE, &[]);
    let created = step(
        client,
        caller,
        journal,
        "playlist.create",
        &create,
        |body| match playlist_urn(body) {
            Some(urn) => format!("created {urn}"),
            None => "the answer names no playlist".to_owned(),
        },
    )
    .await;
    if !created.ok {
        return;
    }
    let Some(urn) = playlist_urn(&created.body) else {
        journal.skip(
            "playlist.left",
            "no playlist urn in the answer; delete the probe playlist by hand",
        );
        return;
    };

    let steady = match track {
        Some(track) => with_track(client, caller, journal, &urn, track).await,
        None => {
            journal.skip("playlist.add", "no --actions-track");
            true
        }
    };
    if !steady {
        journal.skip("playlist.left", LEFT);
        return;
    }

    let delete = playlists::delete(&urn);
    let deleted = step(client, caller, journal, "playlist.delete", &delete, |_| {
        format!("deleted {urn}")
    })
    .await;
    if deleted.blocked {
        journal.skip("playlist.left", LEFT);
        return;
    }
    if !deleted.ok {
        journal.skip(
            "playlist.left",
            &format!("{urn} may be left for the owner to delete by hand"),
        );
        return;
    }
    confirm_gone(client, caller, journal, &urn).await;
}

async fn with_track(
    client: &MobileClient,
    caller: &Caller,
    journal: &mut Journal,
    playlist: &str,
    track: &str,
) -> bool {
    let add = playlists::add_track(playlist, track);
    let added = step(client, caller, journal, "playlist.add", &add, |_| {
        "added".to_owned()
    })
    .await;
    if added.blocked {
        return false;
    }

    let read = playlists::read(playlist);
    let seen = step(client, caller, journal, "playlist.read", &read, |body| {
        format!("{} tracks listed", track_count(body))
    })
    .await;
    if seen.blocked {
        return false;
    }

    let remove = playlists::remove_track(playlist, track);
    let removed = step(client, caller, journal, "playlist.remove", &remove, |_| {
        "removed".to_owned()
    })
    .await;
    !removed.blocked
}

async fn confirm_gone(
    client: &MobileClient,
    caller: &Caller,
    journal: &mut Journal,
    playlist: &str,
) {
    let outcome = client.call(caller, &playlists::read(playlist)).await;
    let blocked = datadome::is_datadome(&outcome);
    let status = outcome.as_ref().ok().map(|reply| reply.status);
    let gone = matches!(status, Some(404 | 410));
    let note = match status {
        Some(code) => format!("read after delete answered {code}"),
        None => "read after delete got no answer".to_owned(),
    };
    journal.event(
        "playlist.gone",
        gone,
        json!({ "status": status, "shape": shape_of(&json_of(&outcome)) }),
        datadome::with_datadome(note, blocked),
    );
}

fn playlist_urn(body: &Value) -> Option<String> {
    let playlist = body.get("playlist").unwrap_or(body);
    if let Some(urn) = playlist.get("urn").and_then(Value::as_str) {
        return Some(urn.to_owned());
    }
    playlist
        .get("id")
        .and_then(Value::as_u64)
        .map(|id| format!("soundcloud:playlists:{id}"))
}

fn track_count(body: &Value) -> usize {
    let playlist = body.get("playlist").unwrap_or(body);
    playlist
        .get("tracks")
        .or_else(|| playlist.get("track_urns"))
        .and_then(Value::as_array)
        .map_or(0, Vec::len)
}

fn shape_of(body: &Option<Value>) -> Vec<String> {
    match body {
        Some(Value::Object(map)) => map.keys().cloned().collect(),
        Some(_) => vec!["not an object".to_owned()],
        None => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn a_playlist_is_known_by_its_urn_or_its_id() {
        assert_eq!(
            playlist_urn(&json!({ "urn": "soundcloud:playlists:1" })).as_deref(),
            Some("soundcloud:playlists:1")
        );
        assert_eq!(
            playlist_urn(&json!({ "playlist": { "id": 2 } })).as_deref(),
            Some("soundcloud:playlists:2")
        );
        assert_eq!(playlist_urn(&json!({})), None);
    }

    #[test]
    fn tracks_are_counted_in_either_answer_shape() {
        assert_eq!(track_count(&json!({ "tracks": [{}, {}] })), 2);
        assert_eq!(
            track_count(&json!({ "playlist": { "track_urns": ["a"] } })),
            1
        );
        assert_eq!(track_count(&json!({})), 0);
    }
}
