use std::time::Duration;

use llm_ladder::{Message, Request, Schema, System};

use super::ResolveArtistRequest;
use super::guess::Guess;
use crate::handlers::metadata::{quoted, quoted_list};

const INSTRUCTIONS: [&str; 10] = [
    "Identify the credited artists of a SoundCloud upload using only the metadata below.",
    "Uploaders are often re-upload channels; the real artist is then usually written in",
    "the title as 'Artist - Title'. Every name you return must appear verbatim in the",
    "metadata. Return null or empty lists when the metadata does not say.",
    "When the title names no artist, the uploader is usually the artist: return the uploader then,",
    "unless the uploader name says it is a channel, label, mix series or fan page posting other people's music.",
    "Answer with exactly one JSON object and nothing else, no markdown and no text around it:",
    r#"{"primary_artist": <name or null>, "featured": [<names>], "producers": [<names>],"#,
    r#""remixers": [<names>], "album": {"title": <name>, "year": <year or null>,"#,
    r#""primary_artist": <name or null>} or null, "confidence": <number from 0 to 1>}"#,
];

pub fn request(track: &ResolveArtistRequest, guess: &Guess, deadline: Duration) -> Request {
    Request {
        system: System::fixed(INSTRUCTIONS.join("\n")),
        messages: vec![Message::user(metadata(track, guess))],
        schema: schema(),
        budget_user: None,
        deadline: Some(deadline),
        plain: None,
        text_field: None,
    }
}

pub fn schema() -> Schema {
    let names = || Schema::array(Schema::String);
    Schema::object([
        ("primary_artist", Schema::String.or_null()),
        ("featured", names()),
        ("producers", names()),
        ("remixers", names()),
        (
            "album",
            Schema::object([
                ("title", Schema::String),
                ("year", Schema::Integer.or_null()),
                ("primary_artist", Schema::String.or_null()),
            ])
            .or_null(),
        ),
        ("confidence", Schema::Number),
    ])
}

fn metadata(track: &ResolveArtistRequest, guess: &Guess) -> String {
    let said = |value: &Option<String>| match value.as_deref() {
        Some(value) if !value.is_empty() => value.to_owned(),
        _ => "-".to_owned(),
    };
    let duration = track
        .duration_ms
        .map_or_else(|| "-".to_owned(), |duration| duration.to_string());
    let primary = guess
        .primary_artist
        .as_deref()
        .map_or_else(|| "None".to_owned(), quoted);
    [
        format!("title: {}", track.title),
        format!("uploader: {}", said(&track.uploader)),
        format!("metadata_artist: {}", said(&track.metadata_artist)),
        format!("isrc: {}", said(&track.isrc)),
        format!("duration_ms: {duration}"),
        format!(
            "parser guess: primary_artist={primary} featured={}",
            quoted_list(&guess.featured)
        ),
        format!("description: {}", said(&track.description)),
    ]
    .join("\n")
}
