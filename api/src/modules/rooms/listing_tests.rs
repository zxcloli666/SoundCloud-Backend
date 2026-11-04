use serde_json::json;

use super::*;
use crate::modules::rooms::model::{
    Backdrop, FREE_CAPACITY, Member, Perks, PlaybackUpdate, Profile,
};

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
        true,
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
    assert_eq!(card.capacity, FREE_CAPACITY);
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
        true,
        0,
    );
    let card = PublicRoom::of(&stranger, &[]).unwrap();
    assert_eq!(card.host_avatar_url, None);
}

#[test]
fn a_full_room_is_marked_and_sinks_below_rooms_with_a_free_seat() {
    let full = room("1", FREE_CAPACITY - 1, 1);
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

#[test]
fn a_card_carries_the_room_title_and_never_the_ban_list() {
    let mut room = room("1", 1, 100);
    room.title = Some("Friday".to_owned());
    room.banned.push("666".to_owned());
    let card = PublicRoom::of(&room, &everyone(&room)).unwrap();
    assert_eq!(card.title.as_deref(), Some("Friday"));
    assert_eq!(card.banned, ["666"]);
    let shown = serde_json::to_value(&card).unwrap();
    assert_eq!(shown["title"], "Friday");
    assert!(shown.get("banned").is_none(), "{shown}");
    assert!(!shown.to_string().contains("666"));
}

#[test]
fn an_unlisted_room_has_no_card() {
    let mut room = room("1", 1, 100);
    room.unlisted = true;
    assert!(PublicRoom::of(&room, &everyone(&room)).is_none());
}

#[test]
fn a_public_room_shows_star_host_cover_and_the_capacity_of_its_perks() {
    let mut star = room("9", 2, 5);
    star.perks = crate::modules::rooms::model::Perks {
        star_host: true,
        capacity: 25,
        mods_max: 8,
    };
    star.cover = Some(crate::modules::rooms::model::Cover {
        url: "https://i1.sndcdn.com/a.jpg".to_owned(),
        accent: 2,
    });
    let value = serde_json::to_value(PublicRoom::of(&star, &[]).unwrap()).unwrap();
    assert_eq!(value["starHost"], json!(true));
    assert_eq!(value["capacity"], json!(25));
    assert_eq!(value["full"], json!(false));
    assert_eq!(value["cover"]["accent"], json!(2));
    let free = serde_json::to_value(PublicRoom::of(&room("8", 0, 6), &[]).unwrap()).unwrap();
    assert_eq!(free["starHost"], json!(false));
    assert_eq!(free["capacity"], json!(10));
    assert_eq!(free["cover"], json!(null));
}

#[test]
fn listeners_past_ten_never_lift_a_star_room_in_the_ranking() {
    let star_room = |host: &str, guests: usize, created_at: i64| {
        let mut room = room(host, 0, created_at);
        room.perks = Perks::star();
        for guest in 0..guests {
            room.join(member(
                &format!("{host}{guest}"),
                "https://i1.sndcdn.com/a.jpg",
            ))
            .unwrap();
        }
        room
    };
    let big = star_room("1", 18, 1);
    let mid = star_room("2", 10, 0);
    let small = star_room("3", 5, 2);
    let mut cards: Vec<PublicRoom> = [&big, &small, &mid]
        .into_iter()
        .filter_map(|room| PublicRoom::of(room, &everyone(room)))
        .collect();
    rank(&mut cards);
    let order: Vec<(&str, usize, bool)> = cards
        .iter()
        .map(|card| (card.host_id.as_str(), card.listeners, card.full))
        .collect();
    assert_eq!(order, [("2", 11, false), ("1", 19, false), ("3", 6, false)]);
}

#[test]
fn the_listing_shows_a_preset_backdrop_but_never_an_image() {
    let mut star = room("9", 1, 5);
    star.perks = Perks::star();
    star.backdrop = Some(Backdrop::Preset {
        id: "vinyl".to_owned(),
    });
    let shown = serde_json::to_value(PublicRoom::of(&star, &[]).unwrap()).unwrap();
    assert_eq!(shown["backdrop"], json!({"kind": "preset", "id": "vinyl"}));
    star.backdrop = Some(Backdrop::Image {
        media_id: uuid::Uuid::from_u128(4),
    });
    let shown = serde_json::to_value(PublicRoom::of(&star, &[]).unwrap()).unwrap();
    assert_eq!(shown["backdrop"], json!(null));
    assert!(
        !shown
            .to_string()
            .contains(&uuid::Uuid::from_u128(4).to_string())
    );
    star.backdrop = Some(Backdrop::Preset {
        id: "vinyl".to_owned(),
    });
    star.perks = Perks::free();
    let lapsed = serde_json::to_value(PublicRoom::of(&star, &[]).unwrap()).unwrap();
    assert_eq!(lapsed["backdrop"], json!(null));
    assert_eq!(lapsed["starHost"], json!(false));
}
