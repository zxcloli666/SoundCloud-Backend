use std::collections::BTreeSet;

use super::normalizer::TitleParts;
use super::*;

fn parsed(title: &str) -> anyhow::Result<TitleParts> {
    Ok(Normalizer::new()?.parse_title(title))
}

fn names(items: &[&str]) -> Vec<String> {
    items.iter().map(|item| (*item).to_owned()).collect()
}

#[test]
fn a_plain_artist_dash_title_is_split() -> anyhow::Result<()> {
    let parts = parsed("Billie Eilish - Ocean Eyes")?;

    assert_eq!(parts.artist.as_deref(), Some("Billie Eilish"));
    assert_eq!(parts.song, "Ocean Eyes");
    Ok(())
}

#[test]
fn every_separator_shape_splits() -> anyhow::Result<()> {
    for dash in ["-", "–", "—", "|"] {
        let parts = parsed(&format!("The Weeknd {dash} Blinding Lights"))?;

        assert_eq!(
            (parts.artist.as_deref(), parts.song.as_str()),
            (Some("The Weeknd"), "Blinding Lights"),
            "{dash}"
        );
    }
    Ok(())
}

#[test]
fn noise_brackets_are_dropped() -> anyhow::Result<()> {
    let parts = parsed("Artist - Song [Official Video] (HD) (Lyrics) [FREE DL]")?;

    assert_eq!(parts.song, "Song");
    assert!(parts.featured.is_empty());
    Ok(())
}

#[test]
fn version_tags_are_kept_aside() -> anyhow::Result<()> {
    let parts = parsed("Post Malone - Sunflower (Slowed + Reverb)")?;
    let slowed_with = parsed("Artist - Song (with reverb slowed)")?;
    let with = parsed("Artist - Song (with Swae Lee)")?;

    assert_eq!(parts.song, "Sunflower");
    assert!(slowed_with.featured.is_empty());
    assert_eq!(with.featured, names(&["Swae Lee"]));
    Ok(())
}

#[test]
fn featured_artist_before_the_dash_is_pulled_out() -> anyhow::Result<()> {
    let parts = parsed("Post Malone ft. Swae Lee - Sunflower")?;

    assert_eq!(parts.artist.as_deref(), Some("Post Malone"));
    assert_eq!(parts.featured, names(&["Swae Lee"]));
    assert_eq!(parts.song, "Sunflower");
    Ok(())
}

#[test]
fn featured_artists_inside_brackets_are_split() -> anyhow::Result<()> {
    let parts = parsed("Artist - Song (feat. A, B & C)")?;

    assert_eq!(parts.featured, names(&["A", "B", "C"]));
    assert_eq!(parts.song, "Song");
    Ok(())
}

#[test]
fn producer_and_remixer_notes_leave_the_song_and_its_featured() -> anyhow::Result<()> {
    let produced = parsed("Artist - Song (prod. Metro Boomin)")?;
    let produced_inline = parsed("Artist - Song prod. by Metro Boomin")?;
    let remixed = parsed("Artist - Song (Skrillex Remix)")?;

    assert_eq!(produced.song, "Song");
    assert!(produced.featured.is_empty());
    assert_eq!(produced_inline.song, "Song");
    assert_eq!(remixed.song, "Song");
    Ok(())
}

#[test]
fn emoji_and_handles_are_stripped() -> anyhow::Result<()> {
    let parts = parsed("Artist - Song 🔥🔥 @promo #trap")?;

    assert_eq!(parts.song, "Song");
    Ok(())
}

#[test]
fn a_title_without_separator_has_no_artist() -> anyhow::Result<()> {
    let parts = parsed("Just A Song Name")?;

    assert_eq!(parts.artist, None);
    assert_eq!(parts.song, "Just A Song Name");
    Ok(())
}

#[test]
fn cyrillic_is_kept_in_the_parsed_title() -> anyhow::Result<()> {
    let parts = parsed("Psychosis — ливень")?;

    assert_eq!(
        (parts.artist.as_deref(), parts.song.as_str()),
        (Some("Psychosis"), "ливень")
    );
    Ok(())
}

#[test]
fn empty_title_does_not_explode() -> anyhow::Result<()> {
    let parts = parsed("")?;

    assert_eq!(parts.artist, None);
    assert_eq!(parts.song, "");
    Ok(())
}

#[test]
fn metadata_artist_credits_split_featured() -> anyhow::Result<()> {
    let credits = Normalizer::new()?.split_credits("Kendrick Lamar feat. SZA");

    assert_eq!(credits.main.as_deref(), Some("Kendrick Lamar"));
    assert_eq!(credits.featured, names(&["SZA"]));
    Ok(())
}

#[test]
fn reupload_channels_are_recognised() {
    for (uploader, channel) in [
        ("Nightcore Vibes", true),
        ("Chill Beats Radio", true),
        ("Trap Nation Records", true),
        ("Psychosis", false),
        ("Billie Eilish", false),
    ] {
        assert_eq!(is_reupload_channel(uploader), channel, "{uploader}");
    }
}

#[test]
fn version_markers() -> anyhow::Result<()> {
    let normalizer = Normalizer::new()?;
    for (title, markers) in [
        ("Song", vec![]),
        ("Song (Skrillex Remix)", vec!["remix"]),
        ("Song (Live)", vec!["live"]),
        ("Live Forever", vec![]),
        ("Song - Sped Up", vec!["sped_up"]),
        ("Song [Nightcore]", vec!["sped_up"]),
        ("Song (slowed + reverb)", vec!["slowed"]),
        ("Song (Acoustic Version)", vec!["acoustic"]),
        ("Song (Instrumental)", vec!["instrumental"]),
    ] {
        assert_eq!(
            normalizer.version_markers(title),
            markers.into_iter().collect::<BTreeSet<_>>(),
            "{title}"
        );
    }
    Ok(())
}

#[test]
fn folding_removes_case_symbols_and_script() {
    assert_eq!(words("  Ocean-Eyes!! "), names(&["ocean", "eyes"]));
    assert_eq!(words("Ливень"), names(&["liven"]));
    assert_eq!(words("Beyoncé"), names(&["beyonce"]));
}

#[test]
fn find_name_needs_the_whole_word_sequence() {
    let text = "Kendrick Lamar - Luther (with SZA)";

    assert_eq!(find_name(text, "SZA").as_deref(), Some("SZA"));
    assert_eq!(
        find_name(text, "kendrick lamar").as_deref(),
        Some("Kendrick Lamar")
    );
    assert_eq!(
        find_name(text, "KENDRICK LAMAR!!").as_deref(),
        Some("Kendrick Lamar")
    );
    assert_eq!(find_name(text, "Lamar Kendrick"), None);
    assert_eq!(find_name(text, "Ken"), None);
    assert_eq!(find_name(text, "!!!"), None);
}

#[test]
fn find_name_does_not_transliterate() {
    assert_eq!(find_name("Zemfira - Iskala", "Земфира"), None);
    assert_eq!(find_name("Земфира - Искала", "Zemfira"), None);
    assert_eq!(find_name("Beyoncé - Halo", "Beyonce"), None);
    assert_eq!(
        find_name("Земфира - Искала", "ИСКАЛА").as_deref(),
        Some("Искала")
    );
}

#[test]
fn case_folds_fully_like_python() {
    assert_eq!(
        find_name("STRASSE - Song", "Straße").as_deref(),
        Some("STRASSE")
    );
    assert_eq!(
        unique(names(&["Straße", "STRASSE", "Other"])),
        names(&["Straße", "Other"])
    );
}

#[test]
fn quoting_reads_like_a_python_repr() {
    assert_eq!(quoted("Kendrick Lamar"), "'Kendrick Lamar'");
    assert_eq!(quoted("it's"), "\"it's\"");
    assert_eq!(quoted("a'b\"c"), "'a\\'b\"c'");
    assert_eq!(quoted("tab\there\\"), "'tab\\there\\\\'");
    assert_eq!(quoted("\u{200b}\u{a0}\u{0}"), "'\\u200b\\xa0\\x00'");
    assert_eq!(quoted("Земфира é"), "'Земфира é'");
    assert_eq!(quoted_list(&names(&["SZA", "it's"])), "['SZA', \"it's\"]");
    assert_eq!(quoted_list(&[]), "[]");
}

#[test]
fn a_share_is_a_finite_number_from_zero_to_one() {
    assert_eq!(share(0.0), Some(0.0));
    assert_eq!(share(1.0), Some(1.0));
    assert_eq!(share(1.01), None);
    assert_eq!(share(-0.1), None);
    assert_eq!(share(f64::NAN), None);
}
