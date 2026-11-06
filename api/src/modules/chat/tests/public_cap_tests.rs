use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

use crate::modules::chat::devices::test_support::{caller, conversation, enroll, fresh_user};
use crate::modules::chat::devices::{self, CAP_PUBLIC_CHAT};
use crate::modules::chat::public_chat::ID;
use crate::modules::chat::public_chat::support::{
    ask_as, bench, error, open, pinging, reset, send, serial, slow_mode, user, version_of,
    with_cap, without_cap,
};

fn ids(answer: &(u16, Value), field: &str) -> Vec<String> {
    answer.1[field]
        .as_array()
        .expect("a list")
        .iter()
        .map(|item| {
            item["id"]
                .as_str()
                .or(item.as_str())
                .unwrap_or_default()
                .to_owned()
        })
        .collect()
}

fn poll(pos: u64, inc: &Value, focus: Option<Uuid>) -> Value {
    json!({"pos": pos, "inc": inc, "hold_ms": 0, "max_bytes": 262_144, "focus": focus})
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn an_old_device_lists_and_syncs_while_the_public_chat_moves(pool: PgPool) {
    let _serial = serial().await;
    let chat = bench(&pool);
    reset(&chat).await;
    slow_mode(&pool, 0).await;
    let (me, friend) = (user(), user());
    let group = conversation(&pool, "group", &[&me, &friend]).await;
    open(&chat, &me).await;
    open(&chat, &friend).await;

    let old = || without_cap(&me);
    let new = || with_cap(&me);
    let first = ask_as(
        &chat,
        old(),
        "POST",
        "/chat/sync",
        Some(poll(0, &Value::Null, None)),
    )
    .await;
    assert_eq!(first.0, 200, "{first:?}");
    let (pos, inc) = (
        first.1["pos"].as_u64().expect("a pos"),
        first.1["inc"].clone(),
    );

    let sent = send(&chat, &friend, pinging(&[&me])).await;
    assert_eq!(sent.0, 200, "{sent:?}");
    assert!(version_of(&chat, &me).await.is_some());

    let listed = ask_as(&chat, old(), "GET", "/chat/conversations", None).await;
    assert_eq!(listed.0, 200, "{listed:?}");
    let kinds: Vec<&str> = listed.1["items"]
        .as_array()
        .expect("items")
        .iter()
        .filter_map(|item| item["kind"].as_str())
        .collect();
    assert_eq!(kinds, ["group"]);
    assert!(ids(&listed, "items").contains(&group.to_string()));

    let old_sync = ask_as(
        &chat,
        old(),
        "POST",
        "/chat/sync",
        Some(poll(pos, &inc, None)),
    )
    .await;
    assert_eq!(old_sync.0, 200, "{old_sync:?}");
    assert!(old_sync.1["pos"].as_u64() > Some(pos));
    assert!(!ids(&old_sync, "convs").contains(&ID.to_string()));
    assert!(!ids(&old_sync, "gone").contains(&ID.to_string()));
    let focused = ask_as(
        &chat,
        old(),
        "POST",
        "/chat/sync",
        Some(poll(pos, &inc, Some(ID))),
    )
    .await;
    assert_eq!(focused.0, 200, "{focused:?}");
    assert!(!ids(&focused, "convs").contains(&ID.to_string()));

    let new_sync = ask_as(
        &chat,
        new(),
        "POST",
        "/chat/sync",
        Some(poll(pos, &inc, None)),
    )
    .await;
    assert_eq!(new_sync.0, 200, "{new_sync:?}");
    assert!(ids(&new_sync, "convs").contains(&ID.to_string()));
    let listed = ask_as(&chat, new(), "GET", "/chat/conversations", None).await;
    assert!(ids(&listed, "items").contains(&ID.to_string()));
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn every_public_route_is_not_found_for_a_device_without_the_cap(pool: PgPool) {
    let _serial = serial().await;
    let chat = bench(&pool);
    reset(&chat).await;
    let me = user();
    open(&chat, &me).await;
    let routes = [
        ("POST", "/chat/public/open".to_owned()),
        ("POST", "/chat/public/watch".to_owned()),
        ("DELETE", "/chat/public/watch".to_owned()),
        ("GET", "/chat/public/log".to_owned()),
        ("GET", format!("/chat/conversations/{ID}")),
        ("PUT", format!("/chat/conversations/{ID}/me")),
        ("GET", format!("/chat/conversations/{ID}/keys")),
        ("GET", format!("/chat/conversations/{ID}/messages")),
        ("POST", format!("/chat/conversations/{ID}/messages")),
        ("GET", format!("/chat/conversations/{ID}/members")),
        ("POST", format!("/chat/conversations/{ID}/typing")),
        ("DELETE", format!("/chat/conversations/{ID}/messages/1")),
    ];
    for (method, uri) in routes {
        let body = (method == "POST" || method == "PUT").then(|| json!({}));
        let answer = ask_as(&chat, without_cap(&me), method, &uri, body.clone()).await;
        assert_eq!(error(&answer), (404, "chat_not_found"), "{method} {uri}");
        let allowed = ask_as(&chat, with_cap(&me), method, &uri, body).await;
        assert_ne!(
            allowed.1["error"],
            json!("chat_not_found"),
            "{method} {uri}"
        );
    }
    let report = ask_as(
        &chat,
        without_cap(&me),
        "POST",
        "/chat/reports",
        Some(json!({"conversation_id": ID, "seq_from": 1, "seq_to": 1, "reason": "spam"})),
    )
    .await;
    assert_eq!(error(&report), (404, "chat_not_found"));
    let urls = ask_as(
        &chat,
        without_cap(&me),
        "POST",
        "/chat/media/urls",
        Some(json!({"conversation_id": ID, "ids": [Uuid::new_v4()]})),
    )
    .await;
    assert_eq!(error(&urls), (404, "chat_not_found"));
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn refreshing_the_device_with_the_cap_shows_the_public_chat(pool: PgPool) {
    let _serial = serial().await;
    let chat = bench(&pool);
    reset(&chat).await;
    let me = fresh_user();
    let device = enroll(&chat, &pool, &me).await;
    assert!(!crate::modules::chat::caller::CallerDevice::from(&device.row).public_chat);
    let old = caller(&me, device.session, Some(&device.row));
    let hidden = ask_as(
        &chat,
        old.clone(),
        "GET",
        &format!("/chat/conversations/{ID}"),
        None,
    )
    .await;
    assert_eq!(error(&hidden), (404, "chat_not_found"));

    let body = json!({"app_version": "9.0.0", "caps": ["embeds", CAP_PUBLIC_CHAT]});
    let refreshed = ask_as(&chat, old, "PUT", "/chat/devices/me", Some(body)).await;
    assert_eq!(refreshed.0, 200, "{refreshed:?}");
    assert_eq!(refreshed.1["app_version"], "9.0.0");

    let row = devices::lookup(&chat, device.keys.id)
        .await
        .expect("a lookup")
        .expect("a device");
    assert_eq!(row.caps, ["embeds", CAP_PUBLIC_CHAT]);
    assert_eq!(row.app_version, "9.0.0");
    let new = caller(&me, device.session, Some(&row));
    assert!(new.sees_public());
    let shown = ask_as(
        &chat,
        new.clone(),
        "GET",
        &format!("/chat/conversations/{ID}"),
        None,
    )
    .await;
    assert_eq!(shown.0, 200, "{shown:?}");
    assert_eq!(shown.1["kind"], "public");
    let listed = ask_as(&chat, new, "GET", "/chat/conversations", None).await;
    assert!(ids(&listed, "items").contains(&ID.to_string()));
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn the_refresh_validates_like_registration_and_skips_identical_writes(pool: PgPool) {
    let _serial = serial().await;
    let chat = bench(&pool);
    let me = fresh_user();
    let device = enroll(&chat, &pool, &me).await;
    let mine = || caller(&me, device.session, Some(&device.row));
    let too_many: Vec<String> = (0..17).map(|n| format!("cap{n}")).collect();
    let bad = [
        json!({"app_version": "9.0.0", "caps": too_many}),
        json!({"app_version": "9.0.0", "caps": ["has space"]}),
        json!({"app_version": "9.0.0", "caps": [""]}),
        json!({"app_version": "9.0.0", "caps": ["x".repeat(33)]}),
        json!({"app_version": "", "caps": []}),
        json!({"app_version": "1".repeat(33), "caps": []}),
    ];
    for body in bad {
        let answer = ask_as(&chat, mine(), "PUT", "/chat/devices/me", Some(body.clone())).await;
        assert_eq!(error(&answer), (400, "chat_invalid"), "{body}");
    }
    let stored: (String, Vec<String>) =
        sqlx::query_as("SELECT app_version, caps FROM chat.devices WHERE id = $1")
            .bind(device.keys.id)
            .fetch_one(&pool)
            .await
            .expect("a device");
    assert_eq!(stored.0, "8.7.0");
    assert_eq!(stored.1, ["embeds"]);

    let same = json!({"app_version": "8.7.0", "caps": ["embeds"]});
    let before = version_of(&chat, &me).await;
    let answer = ask_as(&chat, mine(), "PUT", "/chat/devices/me", Some(same)).await;
    assert_eq!(answer.0, 200, "{answer:?}");
    assert_eq!(version_of(&chat, &me).await, before);

    for _ in 0..30 {
        let body = json!({"app_version": "8.7.0", "caps": ["embeds"]});
        ask_as(&chat, mine(), "PUT", "/chat/devices/me", Some(body)).await;
    }
    let limited = ask_as(
        &chat,
        mine(),
        "PUT",
        "/chat/devices/me",
        Some(json!({"app_version": "8.7.0", "caps": ["embeds"]})),
    )
    .await;
    assert_eq!(limited.0, 429, "{limited:?}");
}
