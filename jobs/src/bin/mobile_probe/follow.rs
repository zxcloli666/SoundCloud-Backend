use sc_mobile::{Caller, MobileClient, follows};
use serde_json::Value;

use crate::actions::{LEFT, is_item, step};
use crate::comment::items_of;
use crate::journal::Journal;

pub async fn run(client: &MobileClient, caller: &Caller, journal: &mut Journal, user: &str) {
    let follow = follows::follow(user);
    let followed = step(client, caller, journal, "follow.create", &follow, |_| {
        "followed".to_owned()
    })
    .await;
    if !followed.ok {
        return;
    }

    let read = follows::followings();
    let read_back = step(client, caller, journal, "follow.read", &read, |body| {
        listed(body, user, true)
    })
    .await;
    if read_back.blocked {
        journal.skip("follow.left", LEFT);
        return;
    }

    let unfollow = follows::unfollow(user);
    let unfollowed = step(client, caller, journal, "follow.delete", &unfollow, |_| {
        "unfollowed".to_owned()
    })
    .await;
    if unfollowed.blocked {
        return;
    }

    step(
        client,
        caller,
        journal,
        "follow.read_after",
        &read,
        |body| listed(body, user, false),
    )
    .await;
}

fn listed(body: &Value, user: &str, expected: bool) -> String {
    let items = items_of(body);
    let found = items.iter().any(|item| is_item(item, user));
    let note = format!(
        "{} followings on the page, user listed: {found}",
        items.len()
    );
    if found == expected {
        note
    } else {
        format!("unclear: {note}")
    }
}
