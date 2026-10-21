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
        false,
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
        crossfade_sec: None,
        next_track: None,
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
    room.set_playback("1", start.clone(), 1_000).unwrap();
    assert_eq!(room.playback.at, 1_000 + MAX_START_LEAD_MS);
    assert_eq!(room.playback.rate, 2.0);
    assert_eq!(room.playback.crossfade_sec, 0);
    start.crossfade_sec = Some(99);
    room.set_playback("1", start.clone(), 100).unwrap();
    assert_eq!(room.playback.crossfade_sec, 12);
    room.set_playback("1", update(PlaybackStatus::Paused, Some("42")), 100)
        .unwrap();
    assert_eq!(room.playback.crossfade_sec, 12);

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

#[test]
fn the_announced_next_track_follows_the_current_one() {
    let mut room = room();
    let mut with_next = update(PlaybackStatus::Playing, Some("42"));
    with_next.next_track = Some(json!({"urn": "soundcloud:tracks:43"}));
    room.set_playback("1", with_next, 100).unwrap();
    assert!(room.playback.next_track.is_some());
    room.set_playback("1", update(PlaybackStatus::Playing, Some("42")), 100)
        .unwrap();
    assert!(room.playback.next_track.is_some());
    room.set_playback("1", update(PlaybackStatus::Playing, Some("44")), 100)
        .unwrap();
    assert!(room.playback.next_track.is_none());
    let mut huge_next = update(PlaybackStatus::Playing, Some("44"));
    huge_next.next_track = Some(json!("x"));
    assert!(room.set_playback("1", huge_next, 100).is_err());
}

#[test]
fn a_room_is_private_until_its_host_opens_it() {
    let mut room = room();
    assert!(!room.public);
    room.join(Member::new("2", &profile("Guest"), 11)).unwrap();
    let denied = room.set_public("2", true).unwrap_err();
    assert_eq!(denied.status(), axum::http::StatusCode::FORBIDDEN);
    room.set_public("1", true).unwrap();
    assert!(room.public);
    room.set_public("1", false).unwrap();
    assert!(!room.public);
    let stored = serde_json::to_value(&room).unwrap();
    let mut legacy = stored.clone();
    legacy.as_object_mut().unwrap().remove("public");
    let decoded: Room = serde_json::from_value(legacy).unwrap();
    assert!(!decoded.public);
}

#[test]
fn a_full_room_gives_the_seats_of_absent_guests_away() {
    let mut room = room();
    for id in 2..=MAX_MEMBERS {
        room.join(Member::new(&id.to_string(), &profile("x"), 0))
            .unwrap();
    }
    room.join(Member::new("50", &profile("fresh"), 900))
        .unwrap_err();
    room.members[1].joined_at = 900;
    room.drop_absent(&["3".to_owned()], 500);
    let left: Vec<&str> = room.members.iter().map(|m| m.user_id.as_str()).collect();
    assert_eq!(left, ["1", "2", "3"]);
    room.join(Member::new("50", &profile("fresh"), 900))
        .unwrap();
}

#[test]
fn room_json_written_before_room_chat_still_loads() {
    let stored = json!({
        "code": "ABC234",
        "hostId": "1",
        "version": 7,
        "createdAt": 10,
        "members": [
            {"userId": "1", "name": "Host", "avatarUrl": null, "joinedAt": 10, "readyUrn": null},
            {"userId": "2", "name": "Guest", "avatarUrl": "https://i1.sndcdn.com/a.jpg",
             "joinedAt": 11, "readyUrn": "soundcloud:tracks:42"}
        ],
        "playback": {"status": "playing", "track": {"title": "Song"},
                     "trackUrn": "soundcloud:tracks:42", "positionMs": 0, "at": 100, "rate": 1.0}
    });
    let room: Room = serde_json::from_value(stored).unwrap();
    assert_eq!(room.version, 7);
    assert_eq!(room.members[1].chat_from_seq, 0);
    assert!(!room.public);
    assert_eq!(room.title, None);
    assert_eq!(room.chat_id, None);
    assert!(room.moderators.is_empty() && room.banned.is_empty() && room.muted.is_empty());
    assert_eq!(room.slow_mode_s, None);
    assert!(room.chat_enabled);
    assert!(room.links);
    assert!(!room.unlisted);
    assert_eq!(room.playback.crossfade_sec, 0);
    let again: Room = serde_json::from_value(serde_json::to_value(&room).unwrap()).unwrap();
    assert_eq!(again, room);
}

#[test]
fn titles_are_one_clean_line_of_at_most_48_chars() {
    assert_eq!(
        clean_title("  Night\n\tDrive  "),
        Some("Night Drive".to_owned())
    );
    assert_eq!(
        clean_title("Lo\u{0007}fi \u{202E}evil\u{200B}"),
        Some("Lofi evil".to_owned())
    );
    assert_eq!(clean_title(" \u{200B} \n"), None);
    let long = "Ж".repeat(60);
    assert_eq!(clean_title(&long).map(|t| t.chars().count()), Some(48));
    assert_eq!(
        clean_title(&format!("{} tail", "a".repeat(47))).as_deref(),
        Some("a".repeat(47).as_str())
    );
}

#[test]
fn a_banned_person_cannot_take_a_seat() {
    let mut room = room();
    room.banned.push("2".to_owned());
    let refused = room
        .join(Member::new("2", &profile("Guest"), 11))
        .unwrap_err();
    assert_eq!(refused.status(), axum::http::StatusCode::FORBIDDEN);
    assert!(!room.is_member("2"));
}

#[test]
fn a_member_keeps_the_point_their_chat_history_starts_at() {
    let mut room = room();
    room.join(Member::new("2", &profile("Guest"), 11).reading_from(40))
        .unwrap();
    room.join(Member::new("2", &profile("Renamed"), 12).reading_from(90))
        .unwrap();
    assert_eq!(room.member("2").map(|m| m.chat_from_seq), Some(40));
    assert_eq!(room.member("2").map(|m| m.joined_at), Some(11));
}
