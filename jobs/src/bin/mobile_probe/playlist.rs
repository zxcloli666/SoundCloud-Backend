use sc_mobile::{Caller, MobileClient, playlists};
use serde_json::Value;

use crate::actions::{Answer, LEFT, step};
use crate::journal::Journal;

const TITLE: &str = "probe";

pub async fn run(
    client: &MobileClient,
    caller: &Caller,
    journal: &mut Journal,
    track: Option<&str>,
) {
    let created = create(client, caller, journal).await;
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

    match track {
        Some(track) => {
            if !with_track(client, caller, journal, &urn, track).await {
                journal.skip("playlist.left", LEFT);
                return;
            }
        }
        None => journal.skip("playlist.add", "no --actions-track"),
    }

    let delete = playlists::delete(&urn);
    let deleted = step(client, caller, journal, "playlist.delete", &delete, |_| {
        format!("deleted {urn}")
    })
    .await;
    if !deleted.ok {
        journal.skip(
            "playlist.left",
            &format!(
                "the notes give no delete shape; {urn} may be left for the owner to delete by hand"
            ),
        );
    }
}

async fn create(client: &MobileClient, caller: &Caller, journal: &mut Journal) -> Answer {
    let mut last = None;
    for (label, request) in playlists::create_variants(TITLE) {
        let name = format!("playlist.create#{label}");
        let answer = step(
            client,
            caller,
            journal,
            &name,
            &request,
            |body| match playlist_urn(body) {
                Some(urn) => format!("created {urn}"),
                None => "the answer names no playlist".to_owned(),
            },
        )
        .await;
        let retry = !answer.ok && !answer.blocked && matches!(answer.status, Some(400 | 422));
        last = Some(answer);
        if !retry {
            break;
        }
    }
    last.unwrap_or(Answer {
        body: Value::Null,
        ok: false,
        blocked: false,
        status: None,
    })
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
    journal.skip(
        "playlist.read",
        "no read: the playlist answer shape is unconfirmed",
    );
    let remove = playlists::remove_track(playlist, track);
    let removed = step(client, caller, journal, "playlist.remove", &remove, |_| {
        "removed".to_owned()
    })
    .await;
    !removed.blocked
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
}
