use serde_json::{Value, json};

use super::*;

fn encoded(event: &SystemEvent) -> Value {
    serde_json::to_value(event).expect("system json")
}

#[test]
fn a_timer_change_names_the_new_value_or_null() {
    let on = SystemEvent::Ttl {
        actor: "101".to_owned(),
        ttl_s: Some(86_400),
    };
    assert_eq!(
        encoded(&on),
        json!({"kind": "ttl", "actor": "101", "ttl_s": 86_400})
    );
    let off = SystemEvent::Ttl {
        actor: "101".to_owned(),
        ttl_s: None,
    };
    assert_eq!(
        encoded(&off),
        json!({"kind": "ttl", "actor": "101", "ttl_s": null})
    );
}

#[test]
fn member_events_carry_actor_and_target() {
    let kicked = SystemEvent::Kicked {
        actor: "101".to_owned(),
        target: "202".to_owned(),
    };
    assert_eq!(
        encoded(&kicked),
        json!({"kind": "kicked", "actor": "101", "target": "202"})
    );
    let role = SystemEvent::Role {
        actor: "101".to_owned(),
        target: "202".to_owned(),
        role: MemberRole::Admin,
    };
    assert_eq!(
        encoded(&role),
        json!({"kind": "role", "actor": "101", "target": "202", "role": "admin"})
    );
    let added = SystemEvent::Added {
        actor: "101".to_owned(),
        targets: vec!["202".to_owned(), "303".to_owned()],
    };
    assert_eq!(
        encoded(&added),
        json!({"kind": "added", "actor": "101", "targets": ["202", "303"]})
    );
}

#[test]
fn the_actor_is_the_row_sender() {
    let deleted = SystemEvent::Deleted {
        actor: "202".to_owned(),
    };
    assert_eq!(deleted.actor(), "202");
    assert_eq!(
        encoded(&deleted),
        json!({"kind": "deleted", "actor": "202"})
    );
}
