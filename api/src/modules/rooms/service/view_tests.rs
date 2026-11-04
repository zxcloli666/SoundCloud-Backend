use serde_json::{Value, json};

use super::*;
use crate::modules::rooms::model::{Mute, Profile};

fn room() -> Room {
    let member = |id: &str| {
        Member::new(
            id,
            &Profile {
                name: format!("User {id}"),
                avatar_url: None,
            },
            10,
        )
        .reading_from(5)
    };
    let mut room = Room::new("ABC234".to_owned(), member("1"), true, 10);
    room.join(member("2")).unwrap();
    room.join(member("3")).unwrap();
    room.moderators.push("2".to_owned());
    room.banned.push("66".to_owned());
    room.title = Some("Friday".to_owned());
    room.muted = vec![
        Mute {
            user_id: "3".to_owned(),
            until_ms: 5_000,
        },
        Mute {
            user_id: "2".to_owned(),
            until_ms: 500,
        },
    ];
    room
}

fn seen_by(viewer: &str) -> Value {
    serde_json::to_value(RoomView::for_viewer(
        room(),
        viewer,
        vec!["1".into()],
        1_000,
    ))
    .unwrap()
}

#[test]
fn only_the_host_and_moderators_see_who_is_banned() {
    assert_eq!(seen_by("1")["banned"], json!(["66"]));
    assert_eq!(seen_by("2")["banned"], json!(["66"]));
    let guest = seen_by("3");
    assert!(guest.get("banned").is_none(), "{guest}");
    assert!(!guest.to_string().contains("66"), "{guest}");
}

#[test]
fn every_member_sees_the_title_moderators_and_live_mutes() {
    let guest = seen_by("3");
    assert_eq!(guest["title"], "Friday");
    assert_eq!(guest["moderators"], json!(["2"]));
    assert_eq!(guest["muted"], json!([{"userId": "3", "untilMs": 5_000}]));
    assert_eq!(guest["slowModeS"], 3);
    assert_eq!(guest["chatEnabled"], true);
    assert_eq!(guest["links"], true);
    assert_eq!(guest["chatId"], Value::Null);
    assert_eq!(guest["serverNow"], 1_000);
    assert_eq!(guest["online"], json!(["1"]));
}

#[test]
fn the_view_is_not_the_stored_room() {
    let guest = seen_by("3");
    assert!(guest["members"][0].get("chatFromSeq").is_none(), "{guest}");
    assert!(guest.get("unlisted").is_none(), "{guest}");
    assert_eq!(
        guest["members"][1],
        json!({"userId": "2", "name": "User 2", "avatarUrl": null, "joinedAt": 10, "readyUrn": null})
    );
}
