use sc_mobile::{Caller, MobileClient, reposts};
use serde_json::Value;

use crate::actions::{LEFT, is_item, step};
use crate::comment::items_of;
use crate::journal::Journal;

pub async fn run(client: &MobileClient, caller: &Caller, journal: &mut Journal, track: &str) {
    let add = reposts::add(track);
    let added = step(client, caller, journal, "repost.add", &add, |_| {
        "reposted".to_owned()
    })
    .await;
    if !added.ok {
        return;
    }

    let read = reposts::posts();
    let read_back = step(client, caller, journal, "repost.read", &read, |body| {
        listed(body, track, true)
    })
    .await;
    if read_back.blocked {
        journal.skip("repost.left", LEFT);
        return;
    }

    let remove = reposts::remove(track);
    let removed = step(client, caller, journal, "repost.remove", &remove, |_| {
        "removed".to_owned()
    })
    .await;
    if removed.blocked {
        return;
    }

    step(
        client,
        caller,
        journal,
        "repost.read_after",
        &read,
        |body| listed(body, track, false),
    )
    .await;
}

fn listed(body: &Value, track: &str, expected: bool) -> String {
    let items = items_of(body);
    let found = items.iter().any(|item| is_item(item, track));
    let note = format!("{} posts on the page, track listed: {found}", items.len());
    if found == expected {
        note
    } else {
        format!("unclear: {note}")
    }
}
