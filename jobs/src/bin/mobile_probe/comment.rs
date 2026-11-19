use sc_mobile::{Caller, MobileClient, comments};
use serde_json::{Value, json};

use crate::journal::{Journal, Sent, failure_note, json_of};

const BODY: &str = "probe";
const TRACK_TIME_MS: u64 = 0;

pub async fn probe(
    client: &MobileClient,
    caller: &Caller,
    journal: &mut Journal,
    track: &str,
    own_urn: Option<&str>,
) {
    let post = comments::post(track, BODY, TRACK_TIME_MS);
    let outcome = client.call(caller, &post).await;
    let posted = json_of(&outcome).as_ref().and_then(comment_urn);
    let note = match (&posted, failure_note(&outcome)) {
        (_, Some(failure)) => failure,
        (Some(urn), None) => format!("created {urn}"),
        (None, None) => "the answer names no comment".to_owned(),
    };
    let refused = matches!(&outcome, Ok(reply) if (400..500).contains(&reply.status));
    journal.reply("comment.post", Sent::of(&post), &outcome, vec![note]);

    let list = comments::list(track);
    let outcome = client.call(caller, &list).await;
    let listed = json_of(&outcome).unwrap_or(Value::Null);
    let items = items_of(&listed);
    let found = posted.clone().or_else(|| {
        items
            .iter()
            .filter(|item| text_of(item, "body") == Some(BODY))
            .filter(|item| own_urn.is_some() && author_urn(item).as_deref() == own_urn)
            .find_map(comment_urn)
    });
    let mut notes = vec![format!("{} comments on the page", items.len())];
    if let Some(urn) = &posted {
        let seen = items
            .iter()
            .any(|item| comment_urn(item).as_ref() == Some(urn));
        notes.push(format!("posted comment listed: {seen}"));
    }
    if let Some(first) = items.first().and_then(Value::as_object) {
        let keys: Vec<&str> = first.keys().map(String::as_str).collect();
        notes.push(format!("item keys {}", keys.join(",")));
    }
    notes.extend(failure_note(&outcome));
    journal.reply("comment.list", Sent::of(&list), &outcome, notes);

    let Some(urn) = found else {
        if refused {
            journal.skip("comment.delete", "the post was refused, nothing to delete");
            return;
        }
        journal.event(
            "comment.delete",
            false,
            json!({ "track": track }),
            vec![
                "the posted comment was not found; check the track and delete it by hand"
                    .to_owned(),
            ],
        );
        return;
    };
    let delete = comments::delete(&urn);
    let outcome = client.call(caller, &delete).await;
    let note = failure_note(&outcome).unwrap_or_else(|| format!("deleted {urn}"));
    journal.reply("comment.delete", Sent::of(&delete), &outcome, vec![note]);
}

fn items_of(body: &Value) -> Vec<Value> {
    let list = body.get("collection").unwrap_or(body);
    list.as_array().cloned().unwrap_or_default()
}

fn comment_urn(item: &Value) -> Option<String> {
    let comment = item.get("comment").unwrap_or(item);
    if let Some(urn) = text_of(comment, "urn") {
        return Some(urn.to_owned());
    }
    comment
        .get("id")
        .and_then(Value::as_u64)
        .map(|id| format!("soundcloud:comments:{id}"))
}

fn author_urn(item: &Value) -> Option<String> {
    ["commenter", "user", "author"]
        .iter()
        .find_map(|key| item.get(key).and_then(|author| text_of(author, "urn")))
        .or_else(|| text_of(item, "user_urn"))
        .map(str::to_owned)
}

fn text_of<'a>(item: &'a Value, key: &str) -> Option<&'a str> {
    item.get(key).and_then(Value::as_str)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_comment_is_known_by_its_urn_or_its_id() {
        assert_eq!(
            comment_urn(&json!({ "urn": "soundcloud:comments:1" })).as_deref(),
            Some("soundcloud:comments:1")
        );
        assert_eq!(
            comment_urn(&json!({ "comment": { "id": 2 } })).as_deref(),
            Some("soundcloud:comments:2")
        );
        assert_eq!(comment_urn(&json!({})), None);
    }

    #[test]
    fn the_author_is_found_under_any_of_the_usual_keys() {
        assert_eq!(
            author_urn(&json!({ "commenter": { "urn": "soundcloud:users:1" } })).as_deref(),
            Some("soundcloud:users:1")
        );
        assert_eq!(
            items_of(&json!({ "collection": [{ "body": "probe" }] })).len(),
            1
        );
    }
}
