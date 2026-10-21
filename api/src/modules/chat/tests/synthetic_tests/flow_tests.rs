use std::collections::BTreeSet;

use redis::AsyncCommands;
use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

use super::support::{Device, call, count, flushed, seed, state};
use crate::modules::chat::rt_keys;

const SEED_DB: u8 = 5;
const SECOND_DB: u8 = 6;
const FLAG_DB: u8 = 7;

fn devices(seeded: &Value) -> Vec<Device> {
    seeded["devices"]
        .as_array()
        .expect("devices")
        .iter()
        .map(Device::from_seeded)
        .collect()
}

fn conversation_of(seeded: &Value, user: &str) -> Uuid {
    seeded["conversations"]
        .as_array()
        .expect("conversations")
        .iter()
        .find(|conversation| {
            conversation["members"]
                .as_array()
                .expect("members")
                .iter()
                .any(|member| member == user)
        })
        .and_then(|conversation| conversation["id"].as_str())
        .and_then(|id| Uuid::parse_str(id).ok())
        .expect("a conversation of the user")
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Redis"]
async fn a_seed_builds_users_devices_and_chats_that_work_through_the_sealed_channel(
    pg: PgPool,
) -> anyhow::Result<()> {
    let redis = flushed(SEED_DB).await;
    let chat = state(pg.clone(), redis.clone(), true);
    let seeded = seed(&chat, 12, 2, 3).await;

    assert_eq!(seeded["first"], 1);
    let devices = devices(&seeded);
    assert_eq!(devices.len(), 24);
    let conversations = seeded["conversations"].as_array().expect("conversations");
    assert_eq!(conversations.len(), 3);
    let members: BTreeSet<&str> = conversations
        .iter()
        .flat_map(|c| c["members"].as_array().expect("members").iter())
        .filter_map(Value::as_str)
        .collect();
    assert_eq!(members.len(), 12);
    for conversation in conversations {
        let size = conversation["members"].as_array().expect("members").len();
        let kind = if size == 2 { "direct" } else { "group" };
        assert_eq!(conversation["kind"], kind, "{conversation}");
    }

    assert_eq!(
        count(
            &pg,
            "SELECT count(*) FROM chat.users WHERE user_id LIKE 'lt:%'"
        )
        .await,
        12
    );
    assert_eq!(
        count(
            &pg,
            "SELECT count(*) FROM chat.devices WHERE user_id LIKE 'lt:%' AND status = 'active' AND history_unlocked_at IS NOT NULL"
        )
        .await,
        24
    );
    assert_eq!(
        count(&pg, "SELECT count(*) FROM chat.conversations").await,
        3
    );
    assert_eq!(
        count(
            &pg,
            "SELECT count(*) FROM chat.members WHERE state = 'active' AND accepted"
        )
        .await,
        12
    );
    let mut conn = redis.get().await?;
    let ttl: i64 = conn
        .ttl(rt_keys::loadtest_session(devices[0].session))
        .await?;
    assert!((1..=7_200).contains(&ttl), "{ttl}");
    drop(conn);

    let device = &devices[0];
    let conversation = conversation_of(&seeded, &device.user);
    let synced = call(
        &chat,
        device,
        "POST",
        "/chat/sync",
        Some(json!({"max_bytes": 65536})),
    )
    .await;
    assert_eq!(synced.status, 200, "{}", synced.body);
    assert_eq!(synced.body["reset"], true);

    let listed = call(&chat, device, "GET", "/chat/conversations", None).await;
    assert_eq!(listed.status, 200, "{}", listed.body);
    let listed_ids: Vec<&str> = listed.body["items"]
        .as_array()
        .expect("items")
        .iter()
        .filter_map(|item| item["id"].as_str())
        .collect();
    assert_eq!(listed_ids, [conversation.to_string()]);

    let keys = call(
        &chat,
        device,
        "GET",
        &format!("/chat/conversations/{conversation}/keys"),
        None,
    )
    .await;
    assert_eq!(keys.status, 200, "{}", keys.body);
    let sealed = crate::modules::chat::b64::decode(keys.body[0]["sealed"].as_str().expect("a key"))
        .expect("base64");
    sc_chat_wire::ck::open_for_device(
        &device.box_secret,
        conversation.as_bytes(),
        0,
        device.id.as_bytes(),
        &sealed,
    )
    .expect("the conversation key opens with the seeded box key");

    let sent = call(
        &chat,
        device,
        "POST",
        &format!("/chat/conversations/{conversation}/messages"),
        Some(json!({
            "client_id": Uuid::now_v7(),
            "class": "post",
            "epoch": 0,
            "body": crate::modules::chat::b64::encode(&[7u8; 296]),
        })),
    )
    .await;
    assert_eq!(sent.status, 200, "{}", sent.body);
    assert_eq!(sent.body["seq"], 1);

    let typing = call(
        &chat,
        device,
        "POST",
        &format!("/chat/conversations/{conversation}/typing"),
        None,
    )
    .await;
    assert_eq!(typing.status, 204, "{}", typing.body);
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Redis"]
async fn a_second_seed_continues_the_numbering_and_both_are_audited(
    pg: PgPool,
) -> anyhow::Result<()> {
    let chat = state(pg.clone(), flushed(SECOND_DB).await, true);
    let first = seed(&chat, 4, 1, 2).await;
    let second = seed(&chat, 6, 1, 1).await;
    assert_eq!(first["first"], 1);
    assert_eq!(second["first"], 5);
    let users: Vec<String> =
        sqlx::query_scalar("SELECT user_id FROM chat.users ORDER BY substr(user_id, 4)::bigint")
            .fetch_all(&pg)
            .await?;
    let expected: Vec<String> = (1..=10).map(|n| format!("lt:{n}")).collect();
    assert_eq!(users, expected);
    assert_eq!(
        count(
            &pg,
            "SELECT count(*) FROM chat.admin_audit WHERE action = 'loadtest_seed' AND admin = 'owner'"
        )
        .await,
        2
    );
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Redis"]
async fn a_synthetic_session_is_never_resolved_once_the_flag_is_off(
    pg: PgPool,
) -> anyhow::Result<()> {
    let redis = flushed(FLAG_DB).await;
    let on = state(pg.clone(), redis.clone(), true);
    let seeded = seed(&on, 2, 1, 1).await;
    let device = &devices(&seeded)[0];

    let open = call(
        &on,
        device,
        "POST",
        "/chat/sync",
        Some(json!({"max_bytes": 4096})),
    )
    .await;
    assert_eq!(open.status, 200, "{}", open.body);

    let off = state(pg, redis, false);
    let refused = call(
        &off,
        device,
        "POST",
        "/chat/sync",
        Some(json!({"max_bytes": 4096})),
    )
    .await;
    assert_eq!(refused.status, 401, "{}", refused.body);
    assert_eq!(refused.body["error"], "chat_relogin_required");
    Ok(())
}
