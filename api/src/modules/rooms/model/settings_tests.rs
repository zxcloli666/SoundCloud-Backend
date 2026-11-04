use axum::http::StatusCode;
use serde_json::{Value, json};
use uuid::Uuid;

use super::*;
use crate::modules::rooms::model::{Member, PRESETS, Perks, Profile};

fn member(id: &str, joined_at: i64) -> Member {
    Member::new(
        id,
        &Profile {
            name: format!("User {id}"),
            avatar_url: None,
        },
        joined_at,
    )
}

fn room(star: bool) -> Room {
    let mut room = Room::new("ABC234".to_owned(), member("1", 10), false, 10);
    room.join(member("2", 20)).unwrap();
    room.perks = Perks::of(star);
    room
}

fn patch(value: Value) -> SettingsPatch {
    serde_json::from_value(value).unwrap()
}

fn refusal(room: &mut Room, value: Value) -> (StatusCode, &'static str) {
    match room.apply_settings("1", &patch(value), 1_000).unwrap_err() {
        AppError::Coded { status, code, .. } => (StatusCode::from_u16(status).unwrap(), code),
        other => panic!("an uncoded refusal: {other:?}"),
    }
}

#[test]
fn settings_belong_to_the_host() {
    let mut room = room(false);
    let patch = patch(json!({
        "title": "  Friday\n night ",
        "slowModeS": 9_999,
        "chat_enabled": false,
        "links": false
    }));
    room.set_moderator("1", "2", true).unwrap();
    assert_eq!(
        room.apply_settings("2", &patch, 0).unwrap_err().status(),
        StatusCode::FORBIDDEN
    );
    room.apply_settings("1", &patch, 0).unwrap();
    assert_eq!(room.title.as_deref(), Some("Friday night"));
    assert_eq!(room.slow_mode_s, Some(MAX_SLOW_MODE_SECS));
    assert!(!room.chat_enabled && !room.links);
    room.apply_settings("1", &SettingsPatch::default(), 0)
        .unwrap();
    assert_eq!(room.title.as_deref(), Some("Friday night"));
    let clear = SettingsPatch {
        title: Some(String::new()),
        ..SettingsPatch::default()
    };
    room.apply_settings("1", &clear, 0).unwrap();
    assert_eq!(room.title, None);
}

#[test]
fn public_rooms_are_slow_by_default() {
    let mut room = room(false);
    assert_eq!(room.slow_mode(), 0);
    room.public = true;
    assert_eq!(room.slow_mode(), PUBLIC_SLOW_MODE_SECS);
    room.slow_mode_s = Some(0);
    assert_eq!(room.slow_mode(), 0);
}

#[test]
fn a_cover_needs_a_star_host_and_a_soundcloud_artwork() {
    let mut free = room(false);
    let cover =
        json!({"cover": {"url": "https://i1.sndcdn.com/artworks-x-t500x500.jpg", "accent": 7}});
    assert_eq!(
        refusal(&mut free, cover.clone()),
        (StatusCode::FORBIDDEN, "room_star_required")
    );
    free.apply_settings("1", &patch(json!({"cover": null})), 0)
        .unwrap();
    let mut star = room(true);
    star.apply_settings("1", &patch(cover), 0).unwrap();
    assert_eq!(
        star.cover,
        Some(Cover {
            url: "https://i1.sndcdn.com/artworks-x-t500x500.jpg".to_owned(),
            accent: 7
        })
    );
    let long = format!("https://i1.sndcdn.com/{}.jpg", "a".repeat(500));
    for bad in [
        json!({"url": "http://i1.sndcdn.com/a.jpg", "accent": 1}),
        json!({"url": "https://sndcdn.com.evil.io/a.jpg", "accent": 1}),
        json!({"url": "https://evilsndcdn.com/a.jpg", "accent": 1}),
        json!({"url": "https://i1.sndcdn.com/a.jpg", "accent": 8}),
        json!({"url": long, "accent": 1}),
        json!({"url": "not a url", "accent": 1}),
    ] {
        assert_eq!(
            refusal(&mut star, json!({"cover": bad})),
            (StatusCode::BAD_REQUEST, "room_invalid")
        );
    }
    star.apply_settings(
        "1",
        &patch(json!({"cover": {"url": "https://sndcdn.com/a.jpg", "accent": 0}})),
        0,
    )
    .unwrap();
    star.apply_settings("1", &patch(json!({"title": "x"})), 0)
        .unwrap();
    assert_eq!(star.cover.as_ref().map(|cover| cover.accent), Some(0));
    star.apply_settings("1", &patch(json!({"cover": null})), 0)
        .unwrap();
    assert_eq!(star.cover, None);
}

#[test]
fn a_lapsed_host_keeps_the_stored_cover_but_nobody_sees_it() {
    let mut star = room(true);
    star.apply_settings(
        "1",
        &patch(json!({"cover": {"url": "https://i1.sndcdn.com/a.jpg", "accent": 2}})),
        0,
    )
    .unwrap();
    assert!(star.cover_shown().is_some());
    star.perks = Perks::free();
    assert!(star.cover.is_some());
    assert_eq!(star.cover_shown(), None);
}

#[test]
fn any_host_picks_the_image_policy_but_never_members_in_public() {
    let mut free = room(false);
    free.apply_settings("1", &patch(json!({"roomImages": "members"})), 0)
        .unwrap();
    assert_eq!(free.room_images, Some(RoomImages::Members));
    free.public = true;
    assert_eq!(
        refusal(&mut free, json!({"room_images": "members"})),
        (StatusCode::BAD_REQUEST, "room_invalid")
    );
    free.apply_settings("1", &patch(json!({"room_images": "off"})), 0)
        .unwrap();
    assert_eq!(free.images_policy(), RoomImages::Off);
}

#[test]
fn a_backdrop_is_a_known_preset_or_an_image_outside_the_cooldown() {
    let mut free = room(false);
    assert_eq!(
        refusal(
            &mut free,
            json!({"backdrop": {"kind": "preset", "id": "aurora"}})
        ),
        (StatusCode::FORBIDDEN, "room_star_required")
    );
    free.apply_settings("1", &patch(json!({"backdrop": null})), 0)
        .unwrap();
    let mut star = room(true);
    for id in PRESETS {
        star.apply_settings(
            "1",
            &patch(json!({"backdrop": {"kind": "preset", "id": id}})),
            0,
        )
        .unwrap();
        assert_eq!(star.backdrop_shown(), star.backdrop_in_public());
    }
    assert_eq!(
        refusal(
            &mut star,
            json!({"backdrop": {"kind": "preset", "id": "lava"}})
        ),
        (StatusCode::BAD_REQUEST, "room_invalid")
    );
    assert!(
        serde_json::from_value::<SettingsPatch>(json!({"backdrop": {"kind": "video"}})).is_err()
    );
    let media = Uuid::from_u128(5);
    star.apply_settings(
        "1",
        &patch(json!({"backdrop": {"kind": "image", "mediaId": media}})),
        0,
    )
    .unwrap();
    assert_eq!(star.image_shown(), Some(media));
    assert_eq!(star.backdrop_in_public(), None);
    assert!(star.clear_image_backdrop(media, Some(2_000)));
    assert_eq!(
        refusal(
            &mut star,
            json!({"backdrop": {"kind": "image", "media_id": media}})
        ),
        (StatusCode::FORBIDDEN, BACKDROP_COOLDOWN)
    );
    star.apply_settings(
        "1",
        &patch(json!({"backdrop": {"kind": "preset", "id": "satin"}})),
        1_000,
    )
    .unwrap();
    star.apply_settings(
        "1",
        &patch(json!({"backdrop": {"kind": "image", "mediaId": media}})),
        2_000,
    )
    .unwrap();
    assert_eq!(star.image_backdrop(), Some(media));
}

#[test]
fn the_backdrop_uses_the_contract_shapes() {
    let media = Uuid::from_u128(9);
    let preset = Backdrop::Preset {
        id: "aurora".to_owned(),
    };
    let image = Backdrop::Image { media_id: media };
    assert_eq!(
        serde_json::to_value(&preset).unwrap(),
        json!({"kind": "preset", "id": "aurora"})
    );
    assert_eq!(
        serde_json::to_value(&image).unwrap(),
        json!({"kind": "image", "mediaId": media})
    );
    assert_eq!(PRESETS.len(), 13);
}
