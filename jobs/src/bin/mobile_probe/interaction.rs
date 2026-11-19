use sc_mobile::interactions::{self, Interaction};
use sc_mobile::{Caller, MobileClient};
use serde_json::Value;

use crate::actions::{LEFT, answer, step};
use crate::journal::Journal;

pub async fn run(
    client: &MobileClient,
    caller: &Caller,
    journal: &mut Journal,
    action: &str,
    interaction: &Interaction<'_>,
) {
    let add = interactions::add(interaction);
    let added = step(
        client,
        caller,
        journal,
        &format!("{action}.add"),
        &add,
        |body| {
            let target = answer(body, "/data/upsertInteraction/targetUrn");
            format!(
                "upserted on {}",
                target.as_deref().unwrap_or("an unnamed target")
            )
        },
    )
    .await;
    if !added.ok {
        return;
    }

    let read = interactions::read(interaction);
    let seen_after_add = step(
        client,
        caller,
        journal,
        &format!("{action}.read"),
        &read,
        |body| seen(body, true),
    )
    .await;
    if seen_after_add.blocked {
        journal.skip(&format!("{action}.left"), LEFT);
        return;
    }

    let remove = interactions::remove(interaction);
    let removed = step(
        client,
        caller,
        journal,
        &format!("{action}.remove"),
        &remove,
        |body| match answer(body, "/data/removeInteraction").as_deref() {
            Some("true") => "removed".to_owned(),
            other => format!(
                "unclear: removeInteraction answered {}",
                other.unwrap_or("nothing")
            ),
        },
    )
    .await;
    if removed.blocked {
        return;
    }

    step(
        client,
        caller,
        journal,
        &format!("{action}.read_after"),
        &read,
        |body| seen(body, false),
    )
    .await;
}

fn seen(body: &Value, expected: bool) -> String {
    let own =
        answer(body, "/data/targetInteractions/0/userInteraction").filter(|value| value != "false");
    let note = format!(
        "own interaction read back: {}",
        own.as_deref().unwrap_or("none")
    );
    if own.is_some() == expected {
        note
    } else {
        format!("unclear: {note}")
    }
}
