use sc_mobile::reactions::{self, TrackReaction};
use sc_mobile::{Caller, MobileClient};

use crate::actions::answer;
use crate::journal::{Journal, Sent, failure_note, json_of};

const AT_SECOND: u64 = 5;

pub async fn probe(client: &MobileClient, caller: &Caller, journal: &mut Journal, track: &str) {
    let reaction = TrackReaction {
        track_urn: track,
        at_second: AT_SECOND,
        value_urn: reactions::FIRE,
    };

    let add = reactions::add(&reaction);
    let outcome = client.call(caller, &add).await;
    let note = failure_note(&outcome).unwrap_or_else(|| {
        format!(
            "upserted on {}",
            json_of(&outcome)
                .and_then(|body| answer(&body, "/data/upsertInteraction/targetUrn"))
                .unwrap_or_else(|| "an unnamed target".to_owned())
        )
    });
    journal.reply("reaction.add", Sent::of(&add), &outcome, vec![note]);

    let read = reactions::read(track, &[AT_SECOND]);
    let outcome = client.call(caller, &read).await;
    let note = failure_note(&outcome).unwrap_or_else(|| {
        format!(
            "own reaction read back: {}",
            json_of(&outcome)
                .and_then(|body| answer(&body, "/data/targetInteractions/0/userInteraction"))
                .unwrap_or_else(|| "none".to_owned())
        )
    });
    journal.reply("reaction.read", Sent::of(&read), &outcome, vec![note]);

    let remove = reactions::remove(&reaction);
    let outcome = client.call(caller, &remove).await;
    let note = failure_note(&outcome).unwrap_or_else(|| {
        format!(
            "removeInteraction answered {}",
            json_of(&outcome)
                .and_then(|body| answer(&body, "/data/removeInteraction"))
                .unwrap_or_else(|| "nothing".to_owned())
        )
    });
    journal.reply("reaction.remove", Sent::of(&remove), &outcome, vec![note]);
}
