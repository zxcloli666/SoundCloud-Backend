use sc_mobile::{Caller, MobileClient, MobileRequest, interactions};
use serde_json::Value;

use crate::args::Args;
use crate::datadome;
use crate::journal::{Journal, Sent, failure_note, json_of};
use crate::{follow, interaction, like, playlist, repost};

pub const LEFT: &str = "stopped on datadome; the action may be left in place, undo it by hand";

pub struct Answer {
    pub body: Value,
    pub ok: bool,
    pub blocked: bool,
    pub status: Option<u16>,
}

pub async fn run(client: &MobileClient, args: &Args, caller: &Caller, journal: &mut Journal) {
    match &args.actions_track {
        Some(track) => {
            like::run(client, caller, journal, track).await;
            repost::run(client, caller, journal, track).await;
        }
        None => {
            journal.skip("like", "no --actions-track");
            journal.skip("repost", "no --actions-track");
        }
    }
    match (&args.comment_urn, &args.actions_track) {
        (Some(comment), Some(track)) => {
            let like = interactions::comment_like(comment, track);
            interaction::run(client, caller, journal, "comment_like", &like).await;
        }
        _ => journal.skip("comment_like", "no --comment-urn"),
    }
    match &args.follow_user {
        Some(user) => follow::run(client, caller, journal, user).await,
        None => journal.skip("follow", "no --follow-user"),
    }
    if args.playlist_probe {
        playlist::run(client, caller, journal, args.actions_track.as_deref()).await;
    } else {
        journal.skip("playlist", "no --playlist-probe");
    }
}

pub async fn step(
    client: &MobileClient,
    caller: &Caller,
    journal: &mut Journal,
    name: &str,
    request: &MobileRequest,
    describe: impl FnOnce(&Value) -> String,
) -> Answer {
    let outcome = client.call(caller, request).await;
    let blocked = datadome::is_datadome(&outcome);
    let status = outcome.as_ref().ok().map(|reply| reply.status);
    let body = json_of(&outcome).unwrap_or(Value::Null);
    let failure = failure_note(&outcome);
    let note = failure.clone().unwrap_or_else(|| describe(&body));
    let notes = datadome::with_datadome(note, blocked);
    journal.reply(name, Sent::of(request), &outcome, notes);
    Answer {
        body,
        ok: failure.is_none(),
        blocked,
        status,
    }
}

pub fn answer(body: &Value, pointer: &str) -> Option<String> {
    match body.pointer(pointer)? {
        Value::Null => None,
        Value::String(text) => Some(text.clone()),
        other => Some(other.to_string()),
    }
}

pub fn is_item(item: &Value, urn: &str) -> bool {
    [Some(item), item.get("target"), item.get("track")]
        .into_iter()
        .flatten()
        .any(|candidate| names(candidate, urn))
}

fn names(item: &Value, urn: &str) -> bool {
    if item.as_str() == Some(urn) {
        return true;
    }
    if ["urn", "target_urn"]
        .iter()
        .any(|key| item.get(key).and_then(Value::as_str) == Some(urn))
    {
        return true;
    }
    item.get("id")
        .and_then(Value::as_u64)
        .is_some_and(|id| urn.rsplit(':').next() == Some(id.to_string().as_str()))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn an_item_is_named_by_its_target_track_or_own_urn() {
        assert!(is_item(
            &json!({ "urn": "soundcloud:users:7" }),
            "soundcloud:users:7"
        ));
        assert!(is_item(
            &json!({ "user": "soundcloud:users:1", "target": "soundcloud:users:7" }),
            "soundcloud:users:7"
        ));
        assert!(is_item(
            &json!({ "user": { "id": 1 }, "target": { "id": 7 } }),
            "soundcloud:users:7"
        ));
        assert!(is_item(
            &json!({ "target_urn": "soundcloud:tracks:3" }),
            "soundcloud:tracks:3"
        ));
        assert!(!is_item(
            &json!({ "user": { "id": 7 }, "target": { "id": 8 } }),
            "soundcloud:users:7"
        ));
    }
}
