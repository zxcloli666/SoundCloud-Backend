use axum::http::{HeaderMap, HeaderValue};

use super::query::{
    Intent, LiveClass, LiveKind, LiveParams, LiveQuery, class_of, identity_of, plain_playlists,
    plain_tracks,
};
use crate::config::LiveMode;
use crate::modules::search::query::{PlaylistSearchQuery, TrackSearchQuery};

fn intent(value: Option<&'static str>) -> Intent {
    let mut headers = HeaderMap::new();
    if let Some(value) = value {
        headers.insert("x-search-intent", HeaderValue::from_static(value));
    }
    Intent::from_headers(&headers)
}

fn plain_query() -> TrackSearchQuery {
    TrackSearchQuery {
        q: Some("lucid dreams".into()),
        ..Default::default()
    }
}

#[test]
fn a_phrase_is_cleaned_bounded_and_hashed_by_its_normal_form() {
    let query = LiveQuery::parse("  Juice\u{0007} WRLD — Lucid Dreams  ").expect("eligible");
    assert_eq!(query.text, "Juice WRLD — Lucid Dreams");
    assert_eq!(query.norm, "juice wrld lucid dreams");
    assert_eq!(query.hash.len(), 32);
    assert_eq!(query.short_hash().len(), 8);
    assert_eq!(
        LiveQuery::parse("juice wrld lucid dreams").map(|other| other.hash),
        Some(query.hash),
        "two spellings of one phrase must share a window"
    );

    let long = "a".repeat(400);
    assert_eq!(LiveQuery::parse(&long).expect("eligible").text.len(), 128);
}

#[test]
fn a_short_query_or_a_link_never_goes_live() {
    for skipped in [
        "",
        "  ",
        "ab",
        "a!",
        "https://soundcloud.com/artist/track",
        "soundcloud.com/artist",
        "snd.sc/abc123",
        "spotify://track/1",
    ] {
        assert_eq!(LiveQuery::parse(skipped), None, "{skipped:?}");
    }
    assert!(LiveQuery::parse("abc").is_some());
    assert!(LiveQuery::parse("кино").is_some());
}

#[test]
fn a_rescue_needs_two_words_or_five_characters() {
    let specific = |raw: &str| LiveQuery::parse(raw).is_some_and(|query| query.is_specific());
    assert!(!specific("abc"));
    assert!(!specific("abcd"));
    assert!(specific("abcde"));
    assert!(specific("ab cd"));
    assert!(specific("u2 one"));
}

#[test]
fn only_a_plain_phrase_is_eligible() {
    assert_eq!(plain_tracks(&plain_query()), Some("lucid dreams"));
    for filtered in [
        TrackSearchQuery {
            ids: Some("1".into()),
            ..plain_query()
        },
        TrackSearchQuery {
            genres: Some("rap".into()),
            ..plain_query()
        },
        TrackSearchQuery {
            tags: Some("x".into()),
            ..plain_query()
        },
        TrackSearchQuery {
            user_urn: Some("soundcloud:users:1".into()),
            ..plain_query()
        },
        TrackSearchQuery {
            access: Some("playable,preview,blocked".into()),
            ..plain_query()
        },
    ] {
        assert_eq!(plain_tracks(&filtered), None, "{filtered:?}");
    }
    assert_eq!(plain_tracks(&TrackSearchQuery::default()), None);

    let playlists = PlaylistSearchQuery {
        q: Some("mix".into()),
        show_tracks: Some("false".into()),
        ..Default::default()
    };
    assert_eq!(plain_playlists(&playlists), Some("mix"));
    let owned = PlaylistSearchQuery {
        q: Some("mix".into()),
        user_urn: Some("soundcloud:users:1".into()),
        ..Default::default()
    };
    assert_eq!(plain_playlists(&owned), None);
}

#[test]
fn the_ym_import_signature_is_recognised_with_or_without_the_header() {
    assert_eq!(
        class_of(LiveKind::Tracks, intent(None), 3, false, true),
        LiveClass::Import,
        "ym.rs sends limit=3&linked_partitioning=true and no page"
    );
    assert_eq!(
        class_of(LiveKind::Tracks, intent(Some("import")), 20, true, false),
        LiveClass::Import
    );
    assert_eq!(
        class_of(LiveKind::Tracks, intent(None), 3, true, true),
        LiveClass::Main,
        "a client that pages is not the importer"
    );
    assert_eq!(
        class_of(LiveKind::Tracks, intent(None), 20, false, true),
        LiveClass::Main
    );
}

#[test]
fn every_request_lands_in_its_class() {
    assert_eq!(intent(None), Intent::Absent);
    assert_eq!(intent(Some("wall")), Intent::Other);
    assert_eq!(
        class_of(LiveKind::Tracks, intent(None), 20, true, false),
        LiveClass::Main
    );
    assert_eq!(
        class_of(LiveKind::Tracks, intent(Some("sc")), 20, true, false),
        LiveClass::Main
    );
    assert_eq!(
        class_of(LiveKind::Tracks, intent(Some("fill")), 20, true, false),
        LiveClass::Fill
    );
    assert_eq!(
        class_of(LiveKind::Users, intent(Some("import")), 3, false, true),
        LiveClass::Side
    );
    assert_eq!(
        class_of(LiveKind::Playlists, intent(None), 20, true, false),
        LiveClass::Side
    );
}

#[test]
fn the_mode_decides_which_classes_may_reach_soundcloud() {
    for class in [
        LiveClass::Main,
        LiveClass::Side,
        LiveClass::Fill,
        LiveClass::Import,
        LiveClass::Rescue,
    ] {
        assert!(!class.allowed_in(LiveMode::Off, true), "{class:?}");
    }
    assert!(LiveClass::Main.allowed_in(LiveMode::Explicit, false));
    assert!(LiveClass::Import.allowed_in(LiveMode::Explicit, false));
    assert!(!LiveClass::Fill.allowed_in(LiveMode::Explicit, true));
    assert!(LiveClass::Fill.allowed_in(LiveMode::Auto, false));
    assert!(!LiveClass::Rescue.allowed_in(LiveMode::Auto, false));
    assert!(LiveClass::Rescue.allowed_in(LiveMode::Auto, true));
}

#[test]
fn classes_carry_the_budgets_of_the_spec() {
    assert!(LiveClass::Main.proxy_allowed());
    assert!(LiveClass::Import.proxy_allowed());
    assert!(!LiveClass::Fill.proxy_allowed());
    assert!(!LiveClass::Rescue.proxy_allowed());
    assert_eq!(LiveClass::Main.budget().as_millis(), 3000);
    assert_eq!(LiveClass::Fill.budget().as_millis(), 2500);
    assert_eq!(LiveClass::Import.budget().as_millis(), 4500);
    assert_eq!(LiveClass::Import.relay_wait().as_millis(), 3000);
    assert_eq!(LiveClass::Main.window_size(LiveKind::Tracks), 40);
    assert_eq!(LiveClass::Side.window_size(LiveKind::Users), 20);
    assert_eq!(LiveClass::Import.window_size(LiveKind::Tracks), 10);
    assert_eq!(
        LiveClass::Fill.scope(LiveKind::Tracks),
        LiveClass::Main.scope(LiveKind::Tracks),
        "the text-mode fill shares the window of the SoundCloud toggle"
    );
    assert_ne!(
        LiveClass::Import.scope(LiveKind::Tracks),
        LiveClass::Main.scope(LiveKind::Tracks),
        "a ten-hit import window must not stand in for a forty-hit toggle window"
    );
    assert_eq!(LiveClass::Main.window_ttl(false), 1800);
    assert_eq!(LiveClass::Main.window_ttl(true), 900);
    assert_eq!(LiveClass::Import.window_ttl(false), 600);
}

#[test]
fn an_identity_is_a_short_hash_of_the_account() {
    let identity = identity_of("12345");
    assert_eq!(identity.len(), 16);
    assert!(!identity.contains("12345"));
    assert_ne!(identity, identity_of("12346"));
}

#[test]
fn linked_partitioning_is_read_as_a_flag() {
    let on = LiveParams {
        linked_partitioning: Some("true".into()),
    };
    let off = LiveParams {
        linked_partitioning: Some("false".into()),
    };
    assert!(on.linked());
    assert!(!off.linked());
    assert!(!LiveParams::default().linked());
}
