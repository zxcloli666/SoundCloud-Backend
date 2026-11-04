use axum::http::StatusCode;
use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

use super::support::{Device, admin, call, count, flushed, redis_keys, seed, state};

const PURGE_DB: u8 = 8;

const SYNTHETIC_ROWS: [&str; 5] = [
    "SELECT count(*) FROM chat.users WHERE user_id LIKE 'lt:%'",
    "SELECT count(*) FROM chat.devices WHERE user_id LIKE 'lt:%'",
    "SELECT count(*) FROM chat.members WHERE user_id LIKE 'lt:%'",
    "SELECT count(*) FROM chat.conversations WHERE created_by LIKE 'lt:%'",
    "SELECT count(*) FROM chat.messages WHERE sender_user LIKE 'lt:%'",
];

async fn real_group_with(pg: &PgPool, synthetic: &str) -> anyhow::Result<Uuid> {
    let id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO chat.conversations (id, kind, created_by, member_count) VALUES ($1, 'group', '101', 2)",
    )
    .bind(id)
    .execute(pg)
    .await?;
    for (user, role) in [("101", "owner"), (synthetic, "member")] {
        sqlx::query(
            "INSERT INTO chat.members (conversation_id, user_id, role, state, accepted) VALUES ($1, $2, $3, 'active', true)",
        )
        .bind(id)
        .bind(user)
        .bind(role)
        .execute(pg)
        .await?;
    }
    Ok(id)
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Redis"]
async fn purge_removes_every_synthetic_row_and_key_and_keeps_real_chats(
    pg: PgPool,
) -> anyhow::Result<()> {
    let redis = flushed(PURGE_DB).await;
    let chat = state(pg.clone(), redis.clone(), true);
    let seeded = seed(&chat, 8, 1, 2).await;
    let devices: Vec<Device> = seeded["devices"]
        .as_array()
        .expect("devices")
        .iter()
        .map(Device::from_seeded)
        .collect();
    for device in &devices {
        let synced = call(
            &chat,
            device,
            "POST",
            "/chat/sync",
            Some(json!({"max_bytes": 4096})),
        )
        .await;
        assert_eq!(synced.status, 200, "{}", synced.body);
    }
    let first: Uuid = seeded["conversations"][0]["id"]
        .as_str()
        .and_then(|id| Uuid::parse_str(id).ok())
        .expect("a conversation");
    let sender = devices
        .iter()
        .find(|device| {
            seeded["conversations"][0]["members"]
                .as_array()
                .expect("members")
                .iter()
                .any(|member| member == &device.user)
        })
        .expect("a member device");
    let keys = call(
        &chat,
        sender,
        "GET",
        &format!("/chat/conversations/{first}/keys"),
        None,
    )
    .await;
    assert_eq!(keys.status, 200, "{}", keys.body);
    for _ in 0..3 {
        let sent = call(
            &chat,
            sender,
            "POST",
            &format!("/chat/conversations/{first}/messages"),
            Some(json!({
                "client_id": Uuid::now_v7(),
                "class": "post",
                "epoch": 0,
                "body": crate::modules::chat::b64::encode(&[1u8; 296]),
            })),
        )
        .await;
        assert_eq!(sent.status, 200, "{}", sent.body);
    }
    let real = real_group_with(&pg, &devices[0].user).await?;
    assert!(!redis_keys(&redis, "chat:v:lt:*").await.is_empty());
    assert!(!redis_keys(&redis, "chat:lt:sess:*").await.is_empty());

    let (status, purged) = admin(&chat, "/chat-admin/loadtest/purge", None).await;
    assert_eq!(status, StatusCode::OK, "{purged}");
    assert_eq!(purged["devices"], 8);
    assert_eq!(purged["sessions"], 8);
    assert_eq!(purged["marked"], 2);
    assert_eq!(purged["purged"], 2);
    assert_eq!(purged["memberships"], 1);
    assert_eq!(purged["users"], 8);
    assert_eq!(purged["pending"], 0);

    for sql in SYNTHETIC_ROWS {
        assert_eq!(count(&pg, sql).await, 0, "{sql}");
    }
    let keys_left: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM chat.conversation_keys WHERE conversation_id = $1",
    )
    .bind(first)
    .fetch_one(&pg)
    .await?;
    assert_eq!(keys_left, 0);
    assert!(redis_keys(&redis, "chat:lt:sess:*").await.is_empty());
    assert!(redis_keys(&redis, "chat:v:lt:*").await.is_empty());
    assert!(redis_keys(&redis, "chat:d:lt:*").await.is_empty());
    assert!(redis_keys(&redis, "chat:lim:*:lt:*").await.is_empty());
    assert!(
        redis_keys(&redis, &format!("chat:c:{first}"))
            .await
            .is_empty()
    );

    let kept: Value = sqlx::query_scalar(
        "SELECT json_build_object('count', member_count, 'deleted', deleted_at IS NOT NULL) FROM chat.conversations WHERE id = $1",
    )
    .bind(real)
    .fetch_one(&pg)
    .await?;
    assert_eq!(kept, json!({"count": 1, "deleted": false}));

    let gone = call(
        &chat,
        &devices[1],
        "POST",
        "/chat/sync",
        Some(json!({"max_bytes": 4096})),
    )
    .await;
    assert_eq!(gone.status, 401, "{}", gone.body);
    assert_eq!(gone.body["error"], "chat_relogin_required");

    let (status, again) = admin(&chat, "/chat-admin/loadtest/purge", None).await;
    assert_eq!(status, StatusCode::OK, "{again}");
    assert_eq!(again["devices"], 0);
    assert_eq!(again["pending"], 0);
    assert_eq!(
        count(
            &pg,
            "SELECT count(*) FROM chat.admin_audit WHERE action = 'loadtest_purge'"
        )
        .await,
        2
    );
    Ok(())
}
