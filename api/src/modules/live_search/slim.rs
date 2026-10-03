use serde_json::{Map, Value};

use super::query::LiveKind;

const TRACK_FIELDS: &[&str] = &[
    "id",
    "urn",
    "kind",
    "title",
    "genre",
    "tag_list",
    "duration",
    "full_duration",
    "artwork_url",
    "permalink_url",
    "waveform_url",
    "sharing",
    "created_at",
    "last_modified",
    "release_date",
    "display_date",
    "language",
    "playback_count",
    "likes_count",
    "reposts_count",
    "comment_count",
    "label_name",
    "license",
];

const PUBLISHER_FIELDS: &[&str] = &["isrc", "artist"];

const UPLOADER_FIELDS: &[&str] = &[
    "id",
    "urn",
    "kind",
    "username",
    "avatar_url",
    "permalink_url",
    "verified",
];

const USER_FIELDS: &[&str] = &[
    "id",
    "urn",
    "kind",
    "username",
    "full_name",
    "first_name",
    "last_name",
    "permalink",
    "permalink_url",
    "avatar_url",
    "country_code",
    "city",
    "verified",
    "followers_count",
    "followings_count",
    "track_count",
    "playlist_count",
    "reposts_count",
    "comments_count",
    "created_at",
    "last_modified",
];

const PLAYLIST_FIELDS: &[&str] = &[
    "id",
    "urn",
    "kind",
    "title",
    "genre",
    "tag_list",
    "artwork_url",
    "permalink",
    "permalink_url",
    "duration",
    "track_count",
    "set_type",
    "is_album",
    "sharing",
    "release_date",
    "display_date",
    "created_at",
    "last_modified",
    "published_at",
    "likes_count",
    "reposts_count",
    "label_name",
    "license",
];

#[derive(Debug, Default, PartialEq)]
pub struct Slimmed {
    pub items: Vec<Value>,
    pub users: Vec<Value>,
}

pub fn slim(kind: LiveKind, raw: &[Value]) -> Slimmed {
    let mut slimmed = Slimmed::default();
    for item in raw {
        let kept = match kind {
            LiveKind::Tracks => track(item),
            LiveKind::Users => user(item),
            LiveKind::Playlists => playlist(item),
        };
        let Some(kept) = kept else {
            continue;
        };
        if urn_of(&kept)
            .is_some_and(|urn| slimmed.items.iter().any(|seen| urn_of(seen) == Some(urn)))
        {
            continue;
        }
        if kind != LiveKind::Users
            && let Some(owner) = item.get("user").and_then(user)
            && !slimmed
                .users
                .iter()
                .any(|seen| urn_of(seen) == urn_of(&owner))
        {
            slimmed.users.push(owner);
        }
        slimmed.items.push(kept);
    }
    slimmed
}

pub fn track(raw: &Value) -> Option<Value> {
    let source = raw.as_object()?;
    if !entity_of(source, "track") || !has_text(source, "title") {
        return None;
    }
    let access = access_of(source)?;
    let mut kept = pick(source, TRACK_FIELDS);
    kept.insert("access".into(), Value::String(access.into()));
    if let Some(publisher) = source.get("publisher_metadata").and_then(Value::as_object) {
        let publisher = pick(publisher, PUBLISHER_FIELDS);
        if let Some(artist) = publisher.get("artist").filter(|artist| is_text(artist)) {
            kept.insert("metadata_artist".into(), artist.clone());
        }
        if !publisher.is_empty() {
            kept.insert("publisher_metadata".into(), Value::Object(publisher));
        }
    }
    if let Some(uploader) = source.get("user").and_then(Value::as_object) {
        let mut uploader = pick(uploader, UPLOADER_FIELDS);
        uploader.insert("kind".into(), Value::String("user".into()));
        kept.insert("user".into(), Value::Object(uploader));
    }
    Some(Value::Object(kept))
}

pub fn user(raw: &Value) -> Option<Value> {
    let source = raw.as_object()?;
    if !entity_of(source, "user") || !has_text(source, "username") {
        return None;
    }
    Some(Value::Object(pick(source, USER_FIELDS)))
}

pub fn playlist(raw: &Value) -> Option<Value> {
    let source = raw.as_object()?;
    if !entity_of(source, "playlist") || !has_text(source, "title") {
        return None;
    }
    let mut kept = pick(source, PLAYLIST_FIELDS);
    if let Some(owner) = source.get("user").and_then(Value::as_object) {
        let mut owner = pick(owner, UPLOADER_FIELDS);
        owner.insert("kind".into(), Value::String("user".into()));
        kept.insert("user".into(), Value::Object(owner));
    }
    Some(Value::Object(kept))
}

pub fn urn_of(item: &Value) -> Option<&str> {
    item.get("urn")
        .and_then(Value::as_str)
        .filter(|urn| !urn.is_empty())
}

fn access_of(source: &Map<String, Value>) -> Option<&'static str> {
    match source.get("policy").and_then(Value::as_str) {
        Some("BLOCK") => None,
        Some("SNIP") => Some("preview"),
        Some(_) => Some("playable"),
        None => match source.get("access").and_then(Value::as_str) {
            Some("blocked") => None,
            Some("preview") => Some("preview"),
            _ => Some("playable"),
        },
    }
}

fn entity_of(source: &Map<String, Value>, kind: &str) -> bool {
    source.get("kind").and_then(Value::as_str) == Some(kind) && has_text(source, "urn")
}

fn has_text(source: &Map<String, Value>, field: &str) -> bool {
    source.get(field).is_some_and(is_text)
}

fn is_text(value: &Value) -> bool {
    value.as_str().is_some_and(|text| !text.trim().is_empty())
}

fn pick(source: &Map<String, Value>, fields: &[&str]) -> Map<String, Value> {
    fields
        .iter()
        .filter_map(|field| {
            source
                .get(*field)
                .filter(|value| !value.is_null())
                .map(|value| ((*field).to_owned(), value.clone()))
        })
        .collect()
}
