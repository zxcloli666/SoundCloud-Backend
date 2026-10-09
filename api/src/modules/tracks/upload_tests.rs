use serde_json::{Map, Value, json};

use super::form::{asset_file_name, track_fields};

fn fields(value: Value) -> Map<String, Value> {
    value.as_object().cloned().expect("object")
}

#[test]
fn a_track_without_a_title_is_refused() {
    assert!(track_fields(fields(json!({"title": "   "}))).is_err());
    assert!(track_fields(fields(json!({"sharing": "public"}))).is_err());
}

#[test]
fn track_fields_are_validated_and_trimmed() {
    let parsed = track_fields(fields(json!({
        "title": "  Night drive ",
        "sharing": "private",
        "tag_list": "synth \"late night\""
    })))
    .expect("valid fields");
    assert!(parsed.contains(&("title".to_owned(), "Night drive".to_owned())));
    assert!(parsed.contains(&("sharing".to_owned(), "private".to_owned())));
    assert!(track_fields(fields(json!({"title": "x", "sharing": "unlisted"}))).is_err());
    assert!(track_fields(fields(json!({"title": "x", "release_date": "2024-13-40"}))).is_err());
}

#[test]
fn audio_file_names_keep_their_format_and_lose_their_path() {
    assert_eq!(
        asset_file_name(Some("C:\\Music\\demo final.WAV")).expect("wav"),
        "demo final.wav"
    );
    assert_eq!(
        asset_file_name(Some("/home/me/mix.flac")).expect("flac"),
        "mix.flac"
    );
    assert_eq!(asset_file_name(Some(".mp3")).expect("bare"), "track.mp3");
}

#[test]
fn unsupported_or_missing_formats_are_refused() {
    assert!(asset_file_name(Some("cover.png")).is_err());
    assert!(asset_file_name(Some("noextension")).is_err());
    assert!(asset_file_name(None).is_err());
}
