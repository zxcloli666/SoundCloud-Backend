use std::collections::HashMap;
use std::time::Duration;

use axum::body::to_bytes;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use chrono::Utc;
use serde_json::{Value, json};
use uuid::Uuid;

use super::test_helpers::{call, caller, caller_on};
use crate::modules::chat::model::{MessageView, Tier};
use crate::modules::chat::sync::changes::{Change, Delta, Versions, decode};
use crate::modules::chat::sync::marks::{Mark, merge, wanted};
use crate::modules::chat::sync::plan::Plan;
use crate::modules::chat::sync::poll::needs_reset;
use crate::modules::chat::sync::reply::{LATER_MAX_MS, SyncReply};
use crate::modules::chat::sync::wire::{RETRY_AFTER_MS, SyncAnswer, SyncRequest};
use crate::modules::chat::sync::{bytes, wire};
use crate::modules::chat::test_support::{config, offline_chat_state};

fn request(body: Value) -> SyncRequest {
    serde_json::from_value(body).expect("a sync request")
}

fn pairs(count: usize) -> Vec<(Uuid, i64)> {
    (0..count)
        .map(|n| (Uuid::from_u128(n as u128), 1))
        .collect()
}

#[test]
fn a_request_over_the_caps_or_with_negative_seqs_is_invalid() {
    let fine = request(json!({"max_bytes": 1000, "known": pairs(64), "read": pairs(64)}));
    assert!(fine.validate().is_ok());
    for broken in [
        json!({"max_bytes": 1000, "known": pairs(65)}),
        json!({"max_bytes": 1000, "read": pairs(65)}),
        json!({"max_bytes": 1000, "delivered": pairs(65)}),
        json!({"max_bytes": 1000, "known": [[Uuid::nil(), -1]]}),
    ] {
        assert!(request(broken).validate().is_err());
    }
}

#[test]
fn hold_and_answer_size_follow_the_tier_and_the_server_cap() {
    let mut config = config(true);
    let body = json!({"hold_ms": 60_000, "max_bytes": 1_000_000});
    let device = Uuid::now_v7();

    let direct = Plan::new(&config, &caller("101", device), &request(body.clone()));
    assert_eq!(direct.hold, Duration::from_millis(wire::HOLD_MAX_MS));
    assert_eq!(direct.budget, wire::ANSWER_BYTES_MAX as usize);

    let pro = Plan::new(
        &config,
        &caller_on("101", device, Tier::Pro),
        &request(body.clone()),
    );
    assert_eq!(pro.hold, Duration::from_millis(wire::HOLD_MAX_PRO_MS));
    assert_eq!(pro.budget, wire::ANSWER_BYTES_MAX_PRO as usize);

    config.hold_max_ms = 5_000;
    let mut tight = caller("101", device);
    tight.max_bytes = 4_096;
    let capped = Plan::new(&config, &tight, &request(body));
    assert_eq!(capped.hold, Duration::from_secs(5));
    assert_eq!(capped.budget, 4_096);

    let short = Plan::new(
        &config,
        &caller("101", device),
        &request(json!({"hold_ms": 0, "max_bytes": 512})),
    );
    assert!(short.hold.is_zero());
    assert_eq!(short.budget, 512);
}

#[test]
fn every_change_kind_survives_the_redis_member_encoding() {
    let conversation = Uuid::now_v7();
    for change in [
        Change::Conversation(conversation),
        Change::Devices,
        Change::Requests,
        Change::GoneMessages(conversation, vec![3, 4, 9]),
    ] {
        assert_eq!(Change::parse(&change.member()), Some(change));
    }
    assert_eq!(Change::parse("rooms:abc"), None);
    assert_eq!(Change::parse(&format!("gone:{conversation}:x")), None);
}

#[test]
fn versions_and_deltas_decode_from_the_script_reply() {
    let conversation = Uuid::now_v7();
    let reply: Vec<String> = [
        "1700000000105",
        "1700000000000",
        "",
        "0",
        &conversation.to_string(),
        "1700000000101",
        "devices",
        "1700000000105",
        "garbage",
        "1700000000105",
    ]
    .iter()
    .map(|item| (*item).to_owned())
    .collect();
    let changes = decode(&reply).expect("a reply");
    assert_eq!(
        changes.versions,
        Versions {
            version: 1_700_000_000_105,
            floor: 1_700_000_000_000,
            requests: None,
            fresh: false,
        }
    );
    assert_eq!(
        changes.deltas,
        vec![
            Delta {
                version: 1_700_000_000_101,
                change: Change::Conversation(conversation),
            },
            Delta {
                version: 1_700_000_000_105,
                change: Change::Devices,
            },
        ]
    );

    let fresh: Vec<String> = ["5", "5", "3", "1"]
        .iter()
        .map(|s| (*s).to_owned())
        .collect();
    let fresh = decode(&fresh).expect("a reply").versions;
    assert!(fresh.fresh);
    assert_eq!(fresh.requests, Some(3));
    assert!(decode(&["1".to_owned()]).is_none());
}

#[test]
fn a_reset_comes_for_a_new_incarnation_a_lost_version_or_a_position_out_of_range() {
    let incarnation = Uuid::now_v7();
    let versions = Versions {
        version: 500,
        floor: 100,
        requests: Some(0),
        fresh: false,
    };
    let at = |pos: u64, inc: Option<Uuid>| request(json!({"pos": pos, "inc": inc, "max_bytes": 1}));
    assert!(!needs_reset(
        &at(100, Some(incarnation)),
        incarnation,
        &versions
    ));
    assert!(!needs_reset(
        &at(500, Some(incarnation)),
        incarnation,
        &versions
    ));
    assert!(needs_reset(
        &at(99, Some(incarnation)),
        incarnation,
        &versions
    ));
    assert!(needs_reset(
        &at(501, Some(incarnation)),
        incarnation,
        &versions
    ));
    assert!(needs_reset(&at(300, None), incarnation, &versions));
    assert!(needs_reset(
        &at(300, Some(Uuid::now_v7())),
        incarnation,
        &versions
    ));
    let lost = Versions {
        fresh: true,
        ..versions
    };
    assert!(needs_reset(&at(300, Some(incarnation)), incarnation, &lost));
}

#[test]
fn marks_keep_the_highest_seq_per_conversation_and_kind() {
    let conversation = Uuid::now_v7();
    let marks = wanted(&request(json!({
        "max_bytes": 1,
        "read": [[conversation, 3], [conversation, 7], [conversation, 0]],
        "delivered": [[conversation, 5]],
    })));
    let found: Vec<(Mark, i64)> = marks.iter().map(|mark| (mark.mark, mark.seq)).collect();
    assert_eq!(found, vec![(Mark::Read, 7), (Mark::Delivered, 5)]);
}

#[test]
fn a_read_mark_counts_as_delivered() {
    let read = HashMap::from([("101".to_owned(), 9_i64), ("202".to_owned(), 2)]);
    let delivered = HashMap::from([("202".to_owned(), 5_i64), ("303".to_owned(), 4)]);
    let merged = merge(&read, &delivered);
    let found: Vec<(&str, i64, i64)> = merged
        .iter()
        .map(|mark| (mark.user_id.as_str(), mark.read_seq, mark.delivered_seq))
        .collect();
    assert_eq!(found, vec![("101", 9, 9), ("202", 2, 5), ("303", 0, 4)]);
}

#[test]
fn the_message_estimate_never_undercounts_the_json() {
    for body_len in [0_usize, 1, 2, 3, 100, 4_096, 48 * 1024] {
        let view = MessageView {
            seq: i64::MAX,
            client_id: Uuid::now_v7(),
            sender: "123456789012".to_owned(),
            sender_device: Some(Uuid::now_v7()),
            class: "system".to_owned(),
            epoch: i32::MAX,
            target_seq: Some(i64::MAX),
            body: (body_len > 0).then(|| vec![255; body_len]),
            ts: Utc::now().timestamp_millis(),
            expires_at: Some(i64::MAX),
        };
        let json = serde_json::to_vec(&view).expect("json");
        assert!(bytes::message(&view) >= json.len(), "{body_len}");
    }
}

async fn reply_parts(reply: SyncReply) -> (StatusCode, Option<u64>, Vec<u8>) {
    let response = reply.into_response();
    let status = response.status();
    let retry = response
        .headers()
        .get(RETRY_AFTER_MS)
        .and_then(|value| value.to_str().ok()?.parse().ok());
    let body = to_bytes(response.into_body(), 1024)
        .await
        .expect("a body")
        .to_vec();
    (status, retry, body)
}

#[tokio::test]
async fn quiet_and_superseded_polls_end_with_an_empty_204() {
    for reply in [SyncReply::Idle, SyncReply::Superseded] {
        assert_eq!(
            reply_parts(reply).await,
            (StatusCode::NO_CONTENT, None, Vec::new())
        );
    }
}

#[tokio::test]
async fn a_later_reply_is_a_204_with_a_jittered_retry_header() {
    let mut seen = Vec::new();
    for _ in 0..50 {
        let (status, retry, body) = reply_parts(SyncReply::later()).await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        assert!(body.is_empty());
        let retry = retry.expect("a retry header");
        assert!(retry <= LATER_MAX_MS);
        seen.push(retry);
    }
    seen.sort_unstable();
    seen.dedup();
    assert!(seen.len() > 10);
}

#[tokio::test]
async fn an_answer_is_json_with_every_field_the_client_reads() {
    let answer = SyncAnswer {
        pos: 7,
        inc: Uuid::nil(),
        reset: false,
        convs: Vec::new(),
        gone: Vec::new(),
        gone_msgs: vec![(Uuid::nil(), 4)],
        devices: Vec::new(),
        requests_count: 2,
    };
    let (status, _, body) = reply_parts(SyncReply::Answer(Box::new(answer))).await;
    assert_eq!(status, StatusCode::OK);
    let body: Value = serde_json::from_slice(&body).expect("json");
    assert_eq!(
        body,
        json!({
            "pos": 7,
            "inc": Uuid::nil(),
            "reset": false,
            "convs": [],
            "gone": [],
            "gone_msgs": [[Uuid::nil(), 4]],
            "devices": [],
            "requests_count": 2,
        })
    );
}

#[tokio::test]
async fn a_sync_without_a_chat_device_or_over_the_caps_is_refused() {
    let chat = offline_chat_state(true);
    let mut deviceless = caller("101", Uuid::now_v7());
    deviceless.device = None;
    let answer = call(
        &chat,
        deviceless,
        "POST",
        "/chat/sync",
        Some(json!({"max_bytes": 1})),
    )
    .await;
    assert_eq!(answer.status, StatusCode::BAD_REQUEST);
    assert_eq!(answer.body, json!({"error": "chat_invalid"}));

    let answer = call(
        &chat,
        caller("101", Uuid::now_v7()),
        "POST",
        "/chat/sync",
        Some(json!({"max_bytes": 1, "known": pairs(65)})),
    )
    .await;
    assert_eq!(answer.body, json!({"error": "chat_invalid"}));

    let answer = call(
        &chat,
        caller("101", Uuid::now_v7()),
        "POST",
        "/chat/sync",
        Some(json!({"pos": 1})),
    )
    .await;
    assert_eq!(answer.body, json!({"error": "chat_invalid"}));
}
