use serde_json::json;

use super::*;
use crate::modules::rooms::model::{Member, PlaybackUpdate, Profile};

fn member(id: &str, avatar: &str) -> Member {
    Member::new(
        id,
        &Profile {
            name: format!("User {id}"),
            avatar_url: Some(avatar.to_owned()),
        },
        10,
    )
}

fn room(host: &str, guests: usize, created_at: i64) -> Room {
    let mut room = Room::new(
        format!("ABC{created_at:03}"),
        member(host, "https://i1.sndcdn.com/avatars-1-large.jpg"),
        created_at,
    );
    for guest in 0..guests {
        room.join(member(
            &format!("{host}{guest}"),
            "https://i1.sndcdn.com/a.jpg",
        ))
        .unwrap();
    }
    room
}

fn everyone(room: &Room) -> Vec<String> {
    room.members.iter().map(|m| m.user_id.clone()).collect()
}

#[test]
fn a_card_shows_the_host_the_track_and_who_is_online() {
    let mut room = room("1", 2, 100);
    room.set_playback(
        "1",
        PlaybackUpdate {
            status: PlaybackStatus::Playing,
            track: Some(json!({
                "title": "  Night Drive  ",
                "artwork_url": null,
                "user": {"username": "Owls", "avatar_url": "https://i1.sndcdn.com/owls.jpg"}
            })),
            track_urn: Some("42".to_owned()),
            position_ms: 0,
            lead_ms: 0,
            rate: None,
            crossfade_sec: None,
            next_track: None,
        },
        200,
    )
    .unwrap();
    let card = PublicRoom::of(&room, &["1".to_owned(), "10".to_owned()]).unwrap();
    assert_eq!(card.host_id, "1");
    assert_eq!(card.host_name, "User 1");
    assert_eq!(card.listeners, 2);
    assert_eq!(card.capacity, MAX_MEMBERS);
    assert!(!card.full);
    let track = card.track.unwrap();
    assert_eq!(track.title, "Night Drive");
    assert_eq!(track.artist.as_deref(), Some("Owls"));
    assert_eq!(
        track.artwork_url.as_deref(),
        Some("https://i1.sndcdn.com/owls.jpg")
    );
    let json = serde_json::to_value(PublicRoom::of(&room, &[]).unwrap()).unwrap();
    assert_eq!(json.get("code"), None);
    assert_eq!(json["listeners"], 1);
}

#[test]
fn only_soundcloud_images_reach_strangers() {
    assert_eq!(image("https://evil.example/pixel.png"), None);
    assert_eq!(image("https://notsndcdn.com/a.jpg"), None);
    assert_eq!(image("http://i1.sndcdn.com/a.jpg"), None);
    assert!(image("https://i1.sndcdn.com/a.jpg").is_some());
    let stranger = Room::new(
        "ABC234".to_owned(),
        member("1", "https://evil.example/a.png"),
        0,
    );
    let card = PublicRoom::of(&stranger, &[]).unwrap();
    assert_eq!(card.host_avatar_url, None);
}

#[test]
fn a_full_room_is_marked_and_sinks_below_rooms_with_a_free_seat() {
    let full = room("1", MAX_MEMBERS - 1, 1);
    let busy = room("2", 3, 2);
    let quiet = room("3", 0, 3);
    let mut cards: Vec<PublicRoom> = [&quiet, &full, &busy]
        .into_iter()
        .filter_map(|room| PublicRoom::of(room, &everyone(room)))
        .collect();
    rank(&mut cards);
    let order: Vec<&str> = cards.iter().map(|c| c.host_id.as_str()).collect();
    assert_eq!(order, ["2", "3", "1"]);
    assert!(cards[2].full);
}
