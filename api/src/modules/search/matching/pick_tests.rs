use serde_json::{Value, json};

use super::pick::{by_uploader, combined, uploaded_by};
use super::wanted::Wanted;

fn hit(n: i64, title: &str, uploader: &str, duration: i64) -> Value {
    json!({
        "urn": format!("soundcloud:tracks:{n}"),
        "title": title,
        "duration": duration,
        "user": {"username": uploader},
    })
}

fn wanted(artist: &str, title: &str, duration: Option<i64>) -> Wanted {
    Wanted::new(artist, title, duration)
}

fn urn(n: i64) -> Option<String> {
    Some(format!("soundcloud:tracks:{n}"))
}

#[test]
fn the_first_close_hit_wins() {
    let hits = [hit(1, "Umbrella", "x", 200_000), hit(2, "Umbrella", "y", 200_500)];
    assert_eq!(combined(&wanted("Rihanna", "Umbrella", Some(200_000)), &hits), urn(1));
}

#[test]
fn duration_within_fifteen_seconds_or_twenty_percent_passes() {
    let w = wanted("A", "Song", Some(200_000));
    assert_eq!(combined(&w, &[hit(1, "Song", "x", 214_000)]), urn(1));
    assert_eq!(combined(&w, &[hit(1, "Song", "x", 180_000)]), urn(1));
    assert_eq!(combined(&w, &[hit(1, "Song", "x", 239_000)]), urn(1));
    assert_eq!(combined(&w, &[hit(1, "Song", "x", 241_000)]), None);
    assert_eq!(combined(&w, &[hit(1, "Song", "x", 1_800_000)]), None);
}

#[test]
fn short_tracks_use_the_fifteen_second_window() {
    let w = wanted("A", "Song", Some(30_000));
    assert_eq!(combined(&w, &[hit(1, "Song", "x", 44_000)]), urn(1));
    assert_eq!(combined(&w, &[hit(1, "Song", "x", 46_000)]), None);
}

#[test]
fn unknown_durations_count_as_close() {
    let hits = [hit(1, "Song", "x", 0)];
    assert_eq!(combined(&wanted("A", "Song", Some(200_000)), &hits), urn(1));
    assert_eq!(
        combined(&wanted("A", "Song", None), &[hit(2, "Song", "x", 999_000)]),
        urn(2)
    );
}

#[test]
fn a_different_title_is_never_returned() {
    let hits = [
        hit(1, "Enter Sandman", "Metallica", 200_000),
        hit(2, "One", "Metallica", 200_000),
    ];
    let w = wanted("Metallica", "Nothing Else Matters", Some(200_000));
    assert_eq!(combined(&w, &hits), None);
    assert_eq!(by_uploader(&w, &hits), None);
}

#[test]
fn titles_match_either_way_around_ignoring_noise() {
    let w = wanted("Rihanna", "Umbrella (Official Video)", None);
    assert_eq!(combined(&w, &[hit(1, "Rihanna - Umbrella ft. Jay-Z", "x", 0)]), urn(1));
    let w = wanted("Rihanna", "Rihanna - Umbrella", None);
    assert_eq!(combined(&w, &[hit(1, "Umbrella", "x", 0)]), urn(1));
}

#[test]
fn very_short_titles_need_an_exact_match() {
    let w = wanted("A", "Go", None);
    assert_eq!(combined(&w, &[hit(1, "Gone Away", "x", 0)]), None);
    assert_eq!(combined(&w, &[hit(1, "Go", "x", 0)]), urn(1));
}

#[test]
fn variants_are_skipped_unless_asked_for() {
    let hits = [
        hit(1, "Umbrella (Sped Up)", "x", 0),
        hit(2, "Umbrella", "y", 0),
    ];
    assert_eq!(combined(&wanted("Rihanna", "Umbrella", None), &hits), urn(2));
    assert_eq!(
        combined(&wanted("Rihanna", "Umbrella (Sped Up)", None), &hits),
        urn(1)
    );
}

#[test]
fn a_hit_uploaded_by_the_artist_beats_an_earlier_one() {
    let hits = [
        hit(1, "Hello", "random reposter", 0),
        hit(2, "Hello", "Adele", 0),
    ];
    assert_eq!(combined(&wanted("Adele", "Hello", None), &hits), urn(2));
}

#[test]
fn the_uploader_search_wants_the_artist_title_and_duration() {
    let hits = [
        hit(1, "Hello", "reposter", 200_000),
        hit(2, "Hello", "Adele", 900_000),
        hit(3, "Hello", "Adele - Official", 200_000),
    ];
    assert_eq!(by_uploader(&wanted("Adele", "Hello", Some(200_000)), &hits), urn(3));
    assert_eq!(by_uploader(&wanted("Lizer", "Hello", Some(200_000)), &hits), None);
}

#[test]
fn uploader_names_follow_the_desktop_rules() {
    assert!(uploaded_by(&hit(1, "t", "Boris Brejcha - Official", 0), "boris brejcha"));
    assert!(uploaded_by(&hit(1, "t", "MØ", 0), "mø"));
    assert!(!uploaded_by(&hit(1, "t", "Kamø", 0), "MØ"));
    assert!(!uploaded_by(&hit(1, "t", "U2 Tribute Band", 0), "U2"));
    assert!(!uploaded_by(&hit(1, "t", "anyone", 0), "!!!"));
    assert!(!uploaded_by(&json!({"urn": "x"}), "Adele"));
}
