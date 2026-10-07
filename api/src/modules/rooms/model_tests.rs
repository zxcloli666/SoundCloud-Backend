use serde_json::json;

use super::*;

fn profile(name: &str) -> Profile {
    Profile {
        name: name.to_owned(),
        avatar_url: Some("https://i1.sndcdn.com/avatar.jpg".to_owned()),
    }
}

fn room() -> Room {
    Room::new(
        "ABC234".to_owned(),
        Member::new("1", &profile("Host"), 10),
        10,
    )
}

fn update(status: PlaybackStatus, urn: Option<&str>) -> PlaybackUpdate {
    PlaybackUpdate {
        status,
        track: urn.map(|urn| json!({"urn": urn, "title": "Song"})),
        track_urn: urn.map(str::to_owned),
        position_ms: 1_500,
        lead_ms: 0,
        rate: None,
    }
}

#[test]
fn codes_use_an_unambiguous_alphabet_and_normalize_on_input() {
    for _ in 0..200 {
        let code = new_code();
        assert_eq!(normalize_code(&code), Some(code.clone()));
        assert!(!code.contains(['0', 'O', '1', 'I']), "{code}");
    }
    assert_eq!(normalize_code(" abc-234 "), Some("ABC234".to_owned()));
    assert_eq!(normalize_code("ABC23"), None);
    assert_eq!(normalize_code("ABC230"), None);
}

#[test]
fn profiles_are_trimmed_and_unsafe_avatars_dropped() {
    let member = Member::new(
        "7",
        &Profile {
            name: "  Name\u{0007}  ".to_owned(),
            avatar_url: Some("http://plain.example/a.png".to_owned()),
        },
        0,
    );
    assert_eq!(member.name, "Name");
    assert_eq!(member.avatar_url, None);
    let blank = Member::new("7", &profile("   "), 0);
    assert_eq!(blank.name, "7");
}

#[test]
fn joining_twice_refreshes_the_profile_and_the_room_has_a_cap() {
    let mut room = room();
    room.join(Member::new("2", &profile("Guest"), 11)).unwrap();
    room.join(Member::new("2", &profile("Renamed"), 12))
        .unwrap();
    assert_eq!(room.members.len(), 2);
    assert_eq!(room.members[1].name, "Renamed");
    for id in 3..=MAX_MEMBERS {
        room.join(Member::new(&id.to_string(), &profile("x"), 0))
            .unwrap();
    }
    let full = room
        .join(Member::new("99", &profile("late"), 0))
        .unwrap_err();
    assert_eq!(full.status(), axum::http::StatusCode::CONFLICT);
}

#[test]
fn only_the_host_moves_playback() {
    let mut room = room();
    room.join(Member::new("2", &profile("Guest"), 11)).unwrap();
    let denied = room
        .set_playback("2", update(PlaybackStatus::Playing, Some("42")), 100)
        .unwrap_err();
    assert_eq!(denied.status(), axum::http::StatusCode::FORBIDDEN);
    room.set_playback("1", update(PlaybackStatus::Playing, Some("42")), 100)
        .unwrap();
    assert_eq!(
        room.playback.track_urn.as_deref(),
        Some("soundcloud:tracks:42")
    );
    assert_eq!(room.playback.position_ms, 1_500);
    assert_eq!(room.playback.at, 100);
}

#[test]
fn a_scheduled_start_is_bounded_and_the_snapshot_is_kept_for_the_same_track() {
    let mut room = room();
    let mut start = update(PlaybackStatus::Playing, Some("soundcloud:tracks:42"));
    start.lead_ms = 60_000;
    start.rate = Some(9.0);
    room.set_playback("1", start, 1_000).unwrap();
    assert_eq!(room.playback.at, 1_000 + MAX_START_LEAD_MS);
    assert_eq!(room.playback.rate, 2.0);

    let mut pause = update(PlaybackStatus::Paused, Some("42"));
    pause.track = None;
    room.set_playback("1", pause, 2_000).unwrap();
    assert_eq!(room.playback.track.as_ref().unwrap()["title"], "Song");

    let mut other = update(PlaybackStatus::Loading, Some("43"));
    other.track = None;
    room.set_playback("1", other, 3_000).unwrap();
    assert_eq!(room.playback.track, None);
}

#[test]
fn playback_rejects_missing_or_foreign_ids_and_oversized_snapshots() {
    let mut room = room();
    let missing = room.set_playback("1", update(PlaybackStatus::Playing, None), 0);
    assert!(missing.is_err());
    let foreign = room.set_playback(
        "1",
        update(PlaybackStatus::Playing, Some("soundcloud:users:42")),
        0,
    );
    assert!(foreign.is_err());
    let mut huge = update(PlaybackStatus::Playing, Some("42"));
    huge.track = Some(json!({"title": "x".repeat(MAX_TRACK_BYTES)}));
    assert!(room.set_playback("1", huge, 0).is_err());
    room.set_playback("1", update(PlaybackStatus::Idle, None), 0)
        .unwrap();
}

#[test]
fn readiness_is_recorded_once_per_track_and_only_for_members() {
    let mut room = room();
    room.join(Member::new("2", &profile("Guest"), 11)).unwrap();
    assert!(room.mark_ready("2", "42").unwrap());
    assert!(!room.mark_ready("2", "soundcloud:tracks:42").unwrap());
    assert_eq!(
        room.members[1].ready_urn.as_deref(),
        Some("soundcloud:tracks:42")
    );
    assert!(room.mark_ready("3", "42").is_err());
}

#[test]
fn removing_reports_whether_anyone_left() {
    let mut room = room();
    room.join(Member::new("2", &profile("Guest"), 11)).unwrap();
    assert!(room.remove("2"));
    assert!(!room.remove("2"));
    assert_eq!(room.members.len(), 1);
}
