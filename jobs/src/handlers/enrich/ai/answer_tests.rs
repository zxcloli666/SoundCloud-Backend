use std::time::Duration;

use serde_json::{Value, json};

use super::ResolveArtistRequest;
use super::answer::{Grounding, ModelAnswer, ResolveArtistData};
use super::guess::deterministic;
use super::prompt;
use crate::handlers::ai_store::RpcSource;
use crate::handlers::metadata::Normalizer;
use crate::llm::test_support::answer_fields;

fn track(title: &str, uploader: Option<&str>, description: Option<&str>) -> ResolveArtistRequest {
    ResolveArtistRequest {
        title: title.to_owned(),
        uploader: uploader.map(str::to_owned),
        metadata_artist: None,
        isrc: None,
        description: description.map(str::to_owned),
        duration_ms: Some(180_000),
    }
}

fn llm_reply(overrides: Value) -> Value {
    let mut reply = json!({
        "primary_artist": "Kendrick Lamar",
        "featured": ["SZA"],
        "producers": [],
        "remixers": [],
        "album": null,
        "confidence": 0.93,
    });
    if let (Some(reply), Some(overrides)) = (reply.as_object_mut(), overrides.as_object()) {
        reply.extend(overrides.clone());
    }
    reply
}

fn grounded(
    track: &ResolveArtistRequest,
    reply: Value,
) -> anyhow::Result<Option<ResolveArtistData>> {
    let guess = deterministic(&Normalizer::new()?, track);
    let reply: ModelAnswer = serde_json::from_value(reply)?;
    Ok(Grounding::new(track, guess.primary_artist.is_some()).answer(&reply))
}

fn names(items: &[&str]) -> Vec<String> {
    items.iter().map(|item| (*item).to_owned()).collect()
}

#[test]
fn every_answer_field_is_named_in_the_prompt() -> anyhow::Result<()> {
    let normalizer = Normalizer::new()?;
    let upload = track("Kendrick Lamar - luther", None, None);
    let system = prompt::request(
        &upload,
        &deterministic(&normalizer, &upload),
        Duration::from_secs(20),
    )
    .system
    .joined();
    let fields = answer_fields(&serde_json::to_value(prompt::schema())?);
    assert_eq!(fields.len(), 9);
    for field in fields {
        assert!(system.contains(&format!("\"{field}\"")), "{field}");
    }
    Ok(())
}

#[test]
fn a_grounded_answer_refines_the_gray_zone() -> anyhow::Result<()> {
    let answer = grounded(
        &track("Kendrick Lamar & SZA - luther", Some("TDE"), None),
        llm_reply(json!({})),
    )?
    .ok_or_else(|| anyhow::anyhow!("rejected"))?;

    assert_eq!(answer.primary_artist.as_deref(), Some("Kendrick Lamar"));
    assert_eq!(answer.featured, names(&["SZA"]));
    assert_eq!(answer.source, RpcSource::Llm);
    Ok(())
}

#[test]
fn names_outside_the_input_are_rejected() -> anyhow::Result<()> {
    let answer = grounded(
        &track("Kendrick Lamar - luther", None, None),
        llm_reply(json!({"primary_artist": "Drake"})),
    )?;

    assert!(answer.is_none());
    Ok(())
}

#[test]
fn an_empty_answer_does_not_erase_the_parsed_artist() -> anyhow::Result<()> {
    let answer = grounded(
        &track("Kendrick Lamar - HUMBLE.", Some("rap hits archive"), None),
        llm_reply(json!({"primary_artist": null, "featured": [], "confidence": 0.2})),
    )?;

    assert!(answer.is_none());
    Ok(())
}

#[test]
fn an_empty_answer_is_kept_when_the_parser_found_nobody() -> anyhow::Result<()> {
    let answer = grounded(
        &track("рассвет", Some("Chill Beats Radio"), None),
        llm_reply(json!({"primary_artist": null, "featured": [], "confidence": 0.2})),
    )?
    .ok_or_else(|| anyhow::anyhow!("rejected"))?;

    assert_eq!(
        (answer.primary_artist, answer.source),
        (None, RpcSource::Llm)
    );
    Ok(())
}

#[test]
fn a_name_spanning_two_fields_is_ungrounded() -> anyhow::Result<()> {
    let answer = grounded(
        &track("Song by Drake", Some("Music Nation"), None),
        llm_reply(json!({"primary_artist": "Drake Music", "featured": []})),
    )?;

    assert!(answer.is_none());
    Ok(())
}

#[test]
fn a_transliterated_name_is_ungrounded() -> anyhow::Result<()> {
    let answer = grounded(
        &track("Zemfira - Iskala", Some("ru hits channel"), None),
        llm_reply(json!({
            "primary_artist": "Земфира",
            "featured": [],
            "album": {"title": "ИСКАЛА", "year": null, "primary_artist": null},
        })),
    )?;

    assert!(answer.is_none());
    Ok(())
}

#[test]
fn llm_names_are_published_as_written_in_the_input() -> anyhow::Result<()> {
    let answer = grounded(
        &track(
            "Kendrick Lamar & SZA - luther",
            None,
            Some("from the album GNX"),
        ),
        llm_reply(json!({
            "primary_artist": "KENDRICK LAMAR!!",
            "featured": ["sza"],
            "album": {"title": "gnx", "year": 2024, "primary_artist": "kendrick  lamar"},
        })),
    )?
    .ok_or_else(|| anyhow::anyhow!("rejected"))?;

    assert_eq!(answer.primary_artist.as_deref(), Some("Kendrick Lamar"));
    assert_eq!(answer.featured, names(&["SZA"]));
    assert_eq!(
        serde_json::to_value(&answer.album)?,
        json!({"title": "GNX", "year": 2024, "primary_artist": "Kendrick Lamar"})
    );
    Ok(())
}

#[test]
fn the_album_title_must_be_grounded_too() -> anyhow::Result<()> {
    let wanted = track("Kendrick Lamar - luther", None, Some("from the album GNX"));
    let album = json!({"title": "GNX", "year": 2024, "primary_artist": "Kendrick Lamar"});

    let grounded_album = grounded(&wanted, llm_reply(json!({"featured": [], "album": album})))?
        .ok_or_else(|| anyhow::anyhow!("rejected"))?;
    let made_up = grounded(
        &wanted,
        llm_reply(
            json!({"featured": [], "album": {"title": "DAMN", "year": null, "primary_artist": null}}),
        ),
    )?;

    assert_eq!(serde_json::to_value(&grounded_album.album)?, album);
    assert!(made_up.is_none());
    Ok(())
}

#[test]
fn an_implausible_album_year_discards_the_answer() -> anyhow::Result<()> {
    let answer = grounded(
        &track("Kendrick Lamar - luther", None, Some("GNX")),
        llm_reply(json!({
            "featured": [],
            "album": {"title": "GNX", "year": 3024, "primary_artist": null},
        })),
    )?;

    assert!(answer.is_none());
    Ok(())
}

#[test]
fn an_album_without_its_optional_fields_is_stored_as_the_worker_stored_it() -> anyhow::Result<()> {
    let answer = grounded(
        &track("Kendrick Lamar - luther", None, Some("GNX")),
        llm_reply(json!({
            "featured": [],
            "album": {"title": "GNX", "year": null, "primary_artist": null},
        })),
    )?
    .ok_or_else(|| anyhow::anyhow!("rejected"))?;

    assert_eq!(
        serde_json::to_value(&answer.album)?,
        json!({"title": "GNX"})
    );
    Ok(())
}

#[test]
fn an_answer_outside_the_schema_is_not_read() {
    for broken in [
        llm_reply(json!({"confidence": true})),
        llm_reply(json!({"featured": "SZA"})),
        llm_reply(json!({"primary_artist": 5})),
        llm_reply(json!({"album": {"title": "GNX", "year": 2024.0, "primary_artist": null}})),
    ] {
        assert!(serde_json::from_value::<ModelAnswer>(broken).is_err());
    }
}

#[test]
fn a_confidence_outside_zero_to_one_is_rejected() -> anyhow::Result<()> {
    let answer = grounded(
        &track("Kendrick Lamar & SZA - luther", None, None),
        llm_reply(json!({"confidence": 1.5})),
    )?;

    assert!(answer.is_none());
    Ok(())
}

#[test]
fn the_answer_schema_is_the_worker_schema() -> anyhow::Result<()> {
    let nullable = |kind: &str| json!({"anyOf": [{"type": kind}, {"type": "null"}]});
    let names = json!({"type": "array", "items": {"type": "string"}});

    assert_eq!(
        serde_json::to_value(prompt::schema())?,
        json!({
            "type": "object",
            "properties": {
                "primary_artist": nullable("string"),
                "featured": names,
                "producers": names,
                "remixers": names,
                "album": {"anyOf": [
                    {
                        "type": "object",
                        "properties": {
                            "title": {"type": "string"},
                            "year": nullable("integer"),
                            "primary_artist": nullable("string"),
                        },
                        "required": ["title", "year", "primary_artist"],
                        "additionalProperties": false,
                    },
                    {"type": "null"},
                ]},
                "confidence": {"type": "number"},
            },
            "required": ["primary_artist", "featured", "producers", "remixers", "album", "confidence"],
            "additionalProperties": false,
        })
    );
    Ok(())
}

#[test]
fn the_prompt_carries_the_worker_words() -> anyhow::Result<()> {
    let normalizer = Normalizer::new()?;
    let titled = track("Kendrick Lamar - luther", None, None);
    let anonymous = ResolveArtistRequest {
        uploader: Some("it's music".to_owned()),
        isrc: Some("USUM72409273".to_owned()),
        description: Some("from GNX".to_owned()),
        duration_ms: None,
        ..track("рассвет", None, None)
    };

    let asked = prompt::request(
        &titled,
        &deterministic(&normalizer, &titled),
        Duration::from_secs(20),
    );
    let unknown = prompt::request(
        &anonymous,
        &deterministic(&normalizer, &anonymous),
        Duration::from_secs(20),
    );

    assert_eq!(
        asked.system.joined(),
        "Identify the credited artists of a SoundCloud upload using only the metadata below.\n\
         Uploaders are often re-upload channels; the real artist is then usually written in\n\
         the title as 'Artist - Title'. Every name you return must appear verbatim in the\n\
         metadata. Return null or empty lists when the metadata does not say.\n\
         Answer with exactly one JSON object and nothing else, no markdown and no text around it:\n\
         {\"primary_artist\": <name or null>, \"featured\": [<names>], \"producers\": [<names>],\n\
         \"remixers\": [<names>], \"album\": {\"title\": <name>, \"year\": <year or null>,\n\
         \"primary_artist\": <name or null>} or null, \"confidence\": <number from 0 to 1>}"
    );
    assert_eq!(
        asked
            .messages
            .first()
            .map(|message| message.content.as_str()),
        Some(
            "title: Kendrick Lamar - luther\n\
             uploader: -\n\
             metadata_artist: -\n\
             isrc: -\n\
             duration_ms: 180000\n\
             parser guess: primary_artist='Kendrick Lamar' featured=[]\n\
             description: -"
        )
    );
    assert_eq!(
        unknown
            .messages
            .first()
            .map(|message| message.content.as_str()),
        Some(
            "title: рассвет\n\
             uploader: it's music\n\
             metadata_artist: -\n\
             isrc: USUM72409273\n\
             duration_ms: -\n\
             parser guess: primary_artist=None featured=[]\n\
             description: from GNX"
        )
    );
    assert_eq!(asked.deadline, Some(Duration::from_secs(20)));
    assert_eq!(asked.budget_user, None);
    Ok(())
}
