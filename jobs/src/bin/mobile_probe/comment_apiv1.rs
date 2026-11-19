use redact::Secret;
use sc_mobile::{Caller, MobileClient, MobileError, MobileReply, comments};
use serde_json::{Value, json};

use crate::apiv1::{Apiv1, Apiv1Request};
use crate::args::Args;
use crate::comment::{BODY, TRACK_TIME_MS, comment_urn, items_of};
use crate::datadome;
use crate::journal::{Journal, Sent, failure_note, json_of};

pub async fn post_then_delete(
    client: &MobileClient,
    args: &Args,
    caller: &Caller,
    journal: &mut Journal,
    track: &str,
) {
    let Some((apiv1, token)) = open(args, caller, journal, "comment.post_apiv1") else {
        return;
    };
    let post = Apiv1Request::post_comment(track, BODY, TRACK_TIME_MS);
    let outcome = apiv1.call(&token, &post).await;
    let posted = json_of(&outcome).as_ref().and_then(comment_urn);
    let note = match (&posted, failure_note(&outcome)) {
        (_, Some(failure)) => failure,
        (Some(urn), None) => format!("created {urn}"),
        (None, None) => "the answer names no comment".to_owned(),
    };
    let blocked = datadome::is_datadome(&outcome);
    let notes = with_datadome(note, blocked);
    journal.reply("comment.post_apiv1", Sent::of_apiv1(&post), &outcome, notes);
    if blocked {
        return;
    }
    let Some(urn) = posted else {
        journal.event(
            "comment.delete",
            false,
            json!({ "track": track }),
            vec!["no comment urn in the answer; check the track and delete it by hand".to_owned()],
        );
        return;
    };
    delete_and_check(client, &apiv1, &token, caller, journal, Some(track), &urn).await;
}

pub async fn delete_only(
    client: &MobileClient,
    args: &Args,
    caller: &Caller,
    journal: &mut Journal,
    comment: &str,
) {
    let Some((apiv1, token)) = open(args, caller, journal, "comment.delete") else {
        return;
    };
    let track = args.comment_track.as_deref();
    delete_and_check(client, &apiv1, &token, caller, journal, track, comment).await;
}

fn open(
    args: &Args,
    caller: &Caller,
    journal: &mut Journal,
    step: &str,
) -> Option<(Apiv1, Secret<String>)> {
    let Some(token) = caller.access_token.clone() else {
        journal.event(step, false, Value::Null, vec!["no access token".to_owned()]);
        return None;
    };
    match Apiv1::new(args.proxy.clone()) {
        Ok(apiv1) => Some((apiv1, token)),
        Err(error) => {
            journal.event(step, false, Value::Null, vec![error.to_string()]);
            None
        }
    }
}

async fn delete_and_check(
    client: &MobileClient,
    apiv1: &Apiv1,
    token: &Secret<String>,
    caller: &Caller,
    journal: &mut Journal,
    track: Option<&str>,
    comment: &str,
) {
    let delete = comments::delete(comment);
    let outcome = client.call(caller, &delete).await;
    let note = failure_note(&outcome).unwrap_or_else(|| format!("deleted {comment}"));
    let blocked = datadome::is_datadome(&outcome);
    journal.reply(
        "comment.delete",
        Sent::of(&delete),
        &outcome,
        with_datadome(note, blocked),
    );
    if blocked {
        return;
    }

    let read = Apiv1Request::get_comment(comment);
    let outcome = apiv1.call(token, &read).await;
    let note = gone_note(&outcome);
    let blocked = datadome::is_datadome(&outcome);
    journal.reply(
        "comment.gone_apiv1",
        Sent::of_apiv1(&read),
        &outcome,
        with_datadome(note, blocked),
    );
    if blocked {
        return;
    }

    let Some(track) = track else {
        journal.skip("comment.list_apiv1", "no --comment-track to list");
        return;
    };
    let list = Apiv1Request::list_comments(track);
    let outcome = apiv1.call(token, &list).await;
    let blocked = datadome::is_datadome(&outcome);
    let notes = listing_notes(&outcome, comment, blocked);
    journal.reply("comment.list_apiv1", Sent::of_apiv1(&list), &outcome, notes);
    if blocked {
        return;
    }

    let list = comments::list(track);
    let outcome = client.call(caller, &list).await;
    let blocked = datadome::is_datadome(&outcome);
    let notes = listing_notes(&outcome, comment, blocked);
    journal.reply("comment.list_mobile", Sent::of(&list), &outcome, notes);
}

fn gone_note(outcome: &Result<MobileReply, MobileError>) -> String {
    match outcome {
        Ok(reply) if matches!(reply.status, 404 | 410) => {
            "gone: the comment is not found".to_owned()
        }
        Ok(reply) if reply.is_success() => "still there: the comment is readable".to_owned(),
        Ok(reply) => format!("inconclusive: answered {}", reply.status),
        Err(error) => error.to_string(),
    }
}

fn listing_notes(
    outcome: &Result<MobileReply, MobileError>,
    comment: &str,
    blocked: bool,
) -> Vec<String> {
    let items = items_of(&json_of(outcome).unwrap_or(Value::Null));
    let listed = items
        .iter()
        .any(|item| comment_urn(item).as_deref() == Some(comment));
    let mut notes = vec![
        format!("{} comments on the page", items.len()),
        format!("deleted comment listed: {listed}"),
    ];
    notes.extend(failure_note(outcome));
    if blocked {
        notes.push(datadome::NOTE.to_owned());
    }
    notes
}

pub fn with_datadome(note: String, blocked: bool) -> Vec<String> {
    let mut notes = vec![note];
    if blocked {
        notes.push(datadome::NOTE.to_owned());
    }
    notes
}
