use super::*;
use crate::modules::rooms::model::{FREE_MODS_MAX, Member, Perks, Profile, STAR_MODS_MAX};

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

fn room() -> Room {
    let mut room = Room::new("ABC234".to_owned(), member("1", 10), false, 10);
    for (id, at) in [("2", 20), ("3", 30), ("4", 40)] {
        room.join(member(id, at)).unwrap();
    }
    room
}

fn status(error: AppError) -> StatusCode {
    error.status()
}

#[test]
fn moderators_kick_guests_but_never_each_other_or_the_host() {
    let mut room = room();
    room.set_moderator("1", "2", true).unwrap();
    room.set_moderator("1", "3", true).unwrap();
    assert_eq!(
        status(room.kick("4", "3").unwrap_err()),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        status(room.kick("2", "3").unwrap_err()),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        status(room.kick("2", "1").unwrap_err()),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        status(room.kick("1", "1").unwrap_err()),
        StatusCode::FORBIDDEN
    );
    room.kick("2", "4").unwrap();
    assert!(!room.is_member("4"));
    assert_eq!(
        status(room.kick("2", "4").unwrap_err()),
        StatusCode::NOT_FOUND
    );
    room.kick("1", "3").unwrap();
    assert!(!room.is_member("3"));
    assert_eq!(room.moderators, ["2"]);
}

#[test]
fn only_the_host_bans_and_a_ban_outlives_the_seat() {
    let mut room = room();
    room.set_moderator("1", "2", true).unwrap();
    assert_eq!(
        status(room.ban("2", "3").unwrap_err()),
        StatusCode::FORBIDDEN
    );
    room.ban("1", "2").unwrap();
    assert!(!room.is_member("2") && room.is_banned("2"));
    assert!(room.moderators.is_empty());
    room.ban("1", "2").unwrap();
    assert_eq!(room.banned, ["2"]);
    room.ban("1", "77").unwrap();
    assert!(room.is_banned("77"));
    assert_eq!(
        status(room.ban("1", "1").unwrap_err()),
        StatusCode::FORBIDDEN
    );
}

#[test]
fn the_ban_list_keeps_the_newest_entries() {
    let mut room = room();
    for id in 0..MAX_BANNED + 5 {
        room.ban("1", &format!("9{id}")).unwrap();
    }
    assert_eq!(room.banned.len(), MAX_BANNED);
    assert_eq!(room.banned.first().map(String::as_str), Some("95"));
}

#[test]
fn only_a_real_user_id_goes_on_the_ban_list() {
    let mut room = room();
    for junk in ["", "lt:", "abc", "1 2", &"9".repeat(4_096)] {
        assert_eq!(
            status(room.ban("1", junk).unwrap_err()),
            StatusCode::NOT_FOUND
        );
    }
    room.ban("1", "lt:42").unwrap();
    assert_eq!(room.banned, ["lt:42"]);
}

#[test]
fn a_mute_ends_on_time_and_can_be_lifted() {
    let mut room = room();
    room.mute("1", "2", 60, 1_000).unwrap();
    assert_eq!(room.muted_until("2", 1_000), Some(61_000));
    assert_eq!(room.muted_until("2", 61_000), None);
    room.mute("1", "2", 0, 2_000).unwrap();
    assert_eq!(room.muted_until("2", 2_000), None);
    room.mute("1", "2", u32::MAX, 0).unwrap();
    assert_eq!(
        room.muted_until("2", 0),
        Some(i64::from(MAX_MUTE_SECS) * 1_000)
    );
    assert_eq!(
        status(room.mute("3", "2", 60, 0).unwrap_err()),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        status(room.mute("1", "99", 60, 0).unwrap_err()),
        StatusCode::NOT_FOUND
    );
}

#[test]
fn a_star_room_takes_eight_moderators_and_a_lapse_removes_none() {
    let mut room = room();
    room.perks = Perks::star();
    for at in 5..=12 {
        room.join(member(&at.to_string(), at * 10)).unwrap();
    }
    let guests: Vec<String> = (2..=9).map(|id| id.to_string()).collect();
    for id in &guests {
        room.set_moderator("1", id, true).unwrap();
    }
    assert_eq!(room.moderators.len(), STAR_MODS_MAX);
    assert_eq!(
        status(room.set_moderator("1", "10", true).unwrap_err()),
        StatusCode::CONFLICT
    );
    room.perks = Perks::free();
    assert_eq!(room.moderators, guests);
    assert_eq!(
        status(room.set_moderator("1", "10", true).unwrap_err()),
        StatusCode::CONFLICT
    );
    for id in ["2", "3", "4", "5", "6"] {
        room.set_moderator("1", id, false).unwrap();
    }
    assert_eq!(
        status(room.set_moderator("1", "10", true).unwrap_err()),
        StatusCode::CONFLICT
    );
    room.set_moderator("1", "7", false).unwrap();
    room.set_moderator("1", "10", true).unwrap();
    assert_eq!(room.moderators, ["8", "9", "10"]);
}

#[test]
fn a_room_has_at_most_three_moderators_and_they_are_members() {
    let mut room = room();
    room.join(member("5", 50)).unwrap();
    for id in ["2", "3", "4"] {
        room.set_moderator("1", id, true).unwrap();
    }
    assert_eq!(
        status(room.set_moderator("1", "5", true).unwrap_err()),
        StatusCode::CONFLICT
    );
    assert_eq!(room.moderators.len(), FREE_MODS_MAX);
    assert_eq!(
        status(room.set_moderator("2", "5", true).unwrap_err()),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        status(room.set_moderator("1", "99", true).unwrap_err()),
        StatusCode::NOT_FOUND
    );
    room.set_moderator("1", "3", false).unwrap();
    room.remove("4");
    assert_eq!(room.moderators, ["2"]);
}

#[test]
fn the_earliest_online_guest_takes_over_the_room() {
    let mut room = room();
    room.set_moderator("1", "3", true).unwrap();
    room.mute("1", "3", 60, 0).unwrap();
    let online = ["4".to_owned(), "3".to_owned()];
    assert_eq!(room.next_host("1", &online).as_deref(), Some("3"));
    assert_eq!(room.hand_off("1", &online).as_deref(), Some("3"));
    assert_eq!(room.host_id, "3");
    assert!(!room.is_member("1"));
    assert!(room.moderators.is_empty());
    assert_eq!(room.muted_until("3", 0), None);
    assert_eq!(room.hand_off("3", &[]), None);
    assert_eq!(room.host_id, "3");
}
