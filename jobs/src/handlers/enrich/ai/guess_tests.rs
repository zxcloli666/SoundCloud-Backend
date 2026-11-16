use backend_contracts::pipeline::ResolveArtistRequest;

use super::guess::{Guess, deterministic};
use crate::handlers::metadata::Normalizer;

fn track(
    title: &str,
    uploader: Option<&str>,
    metadata_artist: Option<&str>,
) -> ResolveArtistRequest {
    ResolveArtistRequest {
        title: title.to_owned(),
        uploader: uploader.map(str::to_owned),
        metadata_artist: metadata_artist.map(str::to_owned),
        isrc: None,
        description: None,
        duration_ms: None,
    }
}

fn guessed(track: &ResolveArtistRequest) -> anyhow::Result<Guess> {
    Ok(deterministic(&Normalizer::new()?, track))
}

fn guess(primary: Option<&str>, featured: &[&str], confidence: f64) -> Guess {
    Guess {
        primary_artist: primary.map(str::to_owned),
        featured: featured.iter().map(|name| (*name).to_owned()).collect(),
        confidence,
    }
}

#[test]
fn metadata_artist_wins_with_its_featured() -> anyhow::Result<()> {
    let found = guessed(&track(
        "luther",
        Some("somebody"),
        Some("Kendrick Lamar ft. SZA"),
    ))?;

    assert_eq!(found, guess(Some("Kendrick Lamar"), &["SZA"], 0.9));
    assert!(found.is_confident());
    Ok(())
}

#[test]
fn title_artist_beats_a_reupload_channel() -> anyhow::Result<()> {
    let found = guessed(&track(
        "Billie Eilish - Ocean Eyes (feat. Finneas) (prod. Finneas)",
        Some("Nightcore Vibes"),
        None,
    ))?;

    assert_eq!(found, guess(Some("Billie Eilish"), &["Finneas"], 0.7));
    assert!(!found.is_confident());
    Ok(())
}

#[test]
fn uploader_is_used_only_when_it_is_not_a_channel() -> anyhow::Result<()> {
    let person = guessed(&track("рассвет", Some("Psychosis"), None))?;
    let channel = guessed(&track("рассвет", Some("Chill Beats Radio"), None))?;
    let nobody = guessed(&track("рассвет", None, None))?;

    assert_eq!(person, guess(Some("Psychosis"), &[], 0.5));
    assert_eq!(channel, guess(None, &[], 0.3));
    assert_eq!(nobody, guess(None, &[], 0.3));
    Ok(())
}

#[test]
fn the_primary_artist_is_not_repeated_as_featured() -> anyhow::Result<()> {
    let found = guessed(&track(
        "luther (feat. KENDRICK LAMAR, SZA, sza)",
        None,
        Some("Kendrick Lamar"),
    ))?;

    assert_eq!(found, guess(Some("Kendrick Lamar"), &["SZA"], 0.9));
    Ok(())
}
