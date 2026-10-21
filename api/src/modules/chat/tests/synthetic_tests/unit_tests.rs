use std::sync::Arc;

use axum::http::StatusCode;
use chat_wire::bind::make_bind;
use chat_wire::prost::Message;
use rand::SeedableRng;
use rand::rngs::StdRng;
use serde_json::json;
use uuid::Uuid;

use super::support::{admin, state};
use crate::modules::chat::conversations::keys::valid_user_id;
use crate::modules::chat::moderation::testing::offline_redis;
use crate::modules::chat::synthetic::layout::{
    GROUP_MAX, GROUP_MIN, SEEDED_DEVICES_MAX, USERS_MAX, check, group_sizes, split, user_id,
};
use crate::modules::chat::synthetic::purge::key_patterns;
use crate::modules::chat::synthetic::sessions::{decode, encode};
use crate::modules::chat::synthetic::{Seed, is_synthetic, session};
use crate::modules::chat::test_support::{config, offline_chat_state};

fn seed(users: u32, devices_per_user: u32, groups: u32) -> Seed {
    Seed {
        users,
        devices_per_user,
        groups,
    }
}

fn lazy_pg() -> sqlx::PgPool {
    sqlx::postgres::PgPoolOptions::new()
        .acquire_timeout(std::time::Duration::from_millis(200))
        .connect_lazy("postgres://scd@127.0.0.1:1/none")
        .expect("a lazy pool")
}

#[test]
fn group_sizes_stay_between_two_and_fifty_and_cover_every_user() {
    let mut rng = StdRng::seed_from_u64(7);
    for (users, groups) in [
        (2, 1),
        (50, 1),
        (100, 2),
        (51, 25),
        (1_000, 40),
        (20_000, 800),
    ] {
        let sizes = group_sizes(users, groups, &mut rng);
        assert_eq!(sizes.len(), groups, "{users} users in {groups}");
        assert_eq!(
            sizes.iter().sum::<usize>(),
            users,
            "{users} users in {groups}"
        );
        assert!(
            sizes
                .iter()
                .all(|size| (GROUP_MIN..=GROUP_MAX).contains(size)),
            "{sizes:?}"
        );
    }
}

#[test]
fn small_groups_and_dms_are_common_in_a_large_seed() {
    let mut rng = StdRng::seed_from_u64(11);
    let sizes = group_sizes(20_000, 1_000, &mut rng);
    let dms = sizes.iter().filter(|size| **size == 2).count();
    let small = sizes.iter().filter(|size| **size <= 10).count();
    assert!(dms > 0, "{sizes:?}");
    assert!(small > sizes.len() / 4, "{small} of {}", sizes.len());
}

#[test]
fn split_cuts_users_in_order_without_gaps() {
    let users: Vec<String> = (1..=7).map(user_id).collect();
    let parts = split(&users, &[2, 5]);
    assert_eq!(parts[0], ["lt:1", "lt:2"]);
    assert_eq!(parts[1], ["lt:3", "lt:4", "lt:5", "lt:6", "lt:7"]);
}

#[test]
fn a_seed_outside_the_limits_is_refused() {
    assert!(check(&seed(2, 1, 1)).is_ok());
    assert!(check(&seed(20_000, 1, 800)).is_ok());
    assert!(check(&seed(10_000, 5, 400)).is_ok());
    for refused in [
        seed(1, 1, 1),
        seed(4, 0, 2),
        seed(4, 11, 2),
        seed(4, 1, 0),
        seed(4, 1, 3),
        seed(101, 1, 2),
        seed(USERS_MAX + 1, 1, 2_000),
        seed(SEEDED_DEVICES_MAX / 2 + 1, 2, 2_000),
    ] {
        assert!(check(&refused).is_err(), "{refused:?}");
    }
}

#[test]
fn synthetic_ids_carry_the_prefix_and_only_pass_in_a_load_test() {
    let id = user_id(42);
    assert_eq!(id, "lt:42");
    assert!(is_synthetic(&id));
    assert!(!is_synthetic("42"));
    assert!(valid_user_id(&id, true));
    assert!(!valid_user_id(&id, false));
}

#[test]
fn chat_opens_to_synthetic_users_only_while_the_load_test_flag_is_on() {
    let mut chat = config(true);
    assert!(!chat.opens_for("lt:1"));
    chat.loadtest = true;
    assert!(chat.opens_for("lt:1"));
    assert!(!chat.opens_for("101"));
    chat.enabled = false;
    assert!(!chat.opens_for("lt:1"));
}

#[test]
fn a_stored_session_opens_only_for_a_synthetic_user_with_a_bind() {
    let auth_seed = [3u8; 32];
    let box_pub = chat_wire::ck::box_public(&[4u8; 32]).expect("a box key");
    let device = Uuid::now_v7();
    let bind = make_bind(&device.to_string(), &auth_seed, &box_pub, 1).encode_to_vec();

    let entry = decode(&encode("lt:9", &bind)).expect("a synthetic session");
    assert_eq!(entry.user_id, "lt:9");
    let caller_bind = entry.bind.expect("a bind");
    assert_eq!(caller_bind.device_id, device);
    assert_eq!(caller_bind.box_pub, box_pub);

    assert!(decode(&encode("9", &bind)).is_none());
    assert!(decode(&encode("lt:9", b"not a bind")).is_none());
    assert!(decode("{}").is_none());
    assert!(decode("lt:9").is_none());
}

#[tokio::test]
async fn a_session_is_never_looked_up_while_the_flag_is_off() {
    let chat = offline_chat_state(true);
    let found = session(&chat, Uuid::now_v7()).await;
    assert!(matches!(found, Ok(None)), "{found:?}");
}

#[tokio::test]
async fn seed_and_purge_are_not_found_while_the_flag_is_off() {
    let chat = state(lazy_pg(), offline_redis(), false);
    for path in ["/chat-admin/loadtest/seed", "/chat-admin/loadtest/purge"] {
        let body = json!({"users": 2, "devices_per_user": 1, "groups": 1});
        let (status, answer) = admin(&chat, path, Some(body)).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{path}");
        assert_eq!(answer, json!({"error": "chat_not_found"}), "{path}");
    }
}

#[tokio::test]
async fn a_malformed_seed_is_refused_before_any_write() {
    let chat: Arc<_> = state(lazy_pg(), offline_redis(), true);
    for body in [
        json!({"users": 2}),
        json!({"users": 1, "devices_per_user": 1, "groups": 1}),
        json!({"users": 300, "devices_per_user": 1, "groups": 2}),
    ] {
        let (status, answer) = admin(&chat, "/chat-admin/loadtest/seed", Some(body)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(answer, json!({"error": "chat_invalid"}));
    }
}

#[test]
fn purge_scans_only_synthetic_keys() {
    assert_eq!(
        key_patterns(),
        [
            "chat:v:lt:*".to_owned(),
            "chat:d:lt:*".to_owned(),
            "chat:lim:*:lt:*".to_owned(),
        ]
    );
}
