use chrono::{TimeZone, Utc};
use serde_json::json;
use uuid::Uuid;

use super::{ConversationKind, MessageRow, MessageView};

#[test]
fn a_message_goes_out_with_base64_body_and_millisecond_times() {
    let row = MessageRow {
        conversation_id: Uuid::nil(),
        seq: 7,
        client_id: Uuid::from_u128(1),
        sender_user: "101".into(),
        sender_device: Some(Uuid::from_u128(2)),
        class: "post".into(),
        epoch: 3,
        target_seq: None,
        body: Some(vec![0xfb, 0xff, 0x01]),
        created_at: Utc.timestamp_millis_opt(1_760_958_000_123).unwrap(),
        expires_at: Some(Utc.timestamp_millis_opt(1_760_961_600_123).unwrap()),
    };
    let wire = serde_json::to_value(MessageView::from(&row)).expect("serializes");
    assert_eq!(
        wire,
        json!({
            "seq": 7,
            "client_id": "00000000-0000-0000-0000-000000000001",
            "sender": "101",
            "sender_device": "00000000-0000-0000-0000-000000000002",
            "class": "post",
            "epoch": 3,
            "target_seq": null,
            "body": "+/8B",
            "ts": 1_760_958_000_123_i64,
            "expires_at": 1_760_961_600_123_i64
        })
    );
    let back: MessageView = serde_json::from_value(wire).expect("deserializes");
    assert_eq!(back.body, row.body);
}

#[test]
fn padded_base64_is_accepted_on_the_way_in() {
    let view: MessageView = serde_json::from_value(json!({
        "seq": 1, "client_id": Uuid::nil(), "sender": "1", "sender_device": null,
        "class": "post", "epoch": 0, "target_seq": null, "body": "AQ==", "ts": 0, "expires_at": null
    }))
    .expect("deserializes");
    assert_eq!(view.body, Some(vec![1]));
}

#[test]
fn saved_messages_are_the_self_kind() {
    assert_eq!(ConversationKind::Saved.as_str(), "self");
    assert_eq!(
        serde_json::to_value(ConversationKind::Saved).unwrap(),
        json!("self")
    );
}
