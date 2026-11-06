use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

use crate::modules::chat::public_chat::ID;
use crate::modules::chat::public_chat::support::{
    OWNER, ask, ask_as, bench, error, open, ping_seq, pinging, post, read_seq, reset, send, serial,
    slow_mode, user, version_of, with_cap,
};
use crate::modules::chat::sync::changes::{self, Change};
use crate::modules::chat::{ChatState, rt_keys};

async fn pinged(chat: &std::sync::Arc<ChatState>, user: &str) -> bool {
    let view = ask(
        chat,
        user,
        "GET",
        &format!("/chat/conversations/{ID}"),
        None,
    )
    .await;
    assert_eq!(view.0, 200, "{view:?}");
    view.1["pinged"].as_bool().expect("a pinged flag")
}

async fn deltas_of(chat: &ChatState, user: &str) -> Vec<Change> {
    let floor = changes::read(chat, user, 0)
        .await
        .expect("versions")
        .versions
        .floor;
    changes::read(chat, user, floor)
        .await
        .expect("changes")
        .deltas
        .into_iter()
        .map(|delta| delta.change)
        .collect()
}

async fn set_counter(chat: &ChatState, user: &str, value: u64) {
    let key = rt_keys::limit("public_ping:10m", user);
    let mut conn = chat.redis.get().await.expect("a connection");
    let _: () = redis::cmd("SET")
        .arg(&key)
        .arg(value)
        .arg("EX")
        .arg(600)
        .query_async(&mut conn)
        .await
        .expect("a counter");
}

async fn counter(chat: &ChatState, user: &str) -> Option<u64> {
    let mut conn = chat.redis.get().await.expect("a connection");
    redis::cmd("GET")
        .arg(rt_keys::limit("public_ping:10m", user))
        .query_async(&mut conn)
        .await
        .expect("a counter")
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn a_ping_bumps_a_member_who_is_not_watching(pool: PgPool) {
    let _serial = serial().await;
    let chat = bench(&pool);
    reset(&chat).await;
    slow_mode(&pool, 0).await;
    let (author, target, bystander) = (user(), user(), user());
    for who in [&author, &target, &bystander] {
        open(&chat, who).await;
    }
    let sent = send(&chat, &author, pinging(&[&target])).await;
    assert_eq!(sent.0, 200, "{sent:?}");
    let seq = sent.1["seq"].as_i64().expect("a seq");
    assert!(version_of(&chat, &target).await.is_some());
    assert!(
        deltas_of(&chat, &target)
            .await
            .contains(&Change::Conversation(ID))
    );
    assert_eq!(version_of(&chat, &bystander).await, None);
    assert_eq!(ping_seq(&pool, &target).await, seq);
    assert_eq!(ping_seq(&pool, &bystander).await, 0);
    assert!(pinged(&chat, &target).await);
    assert!(!pinged(&chat, &bystander).await);
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn pings_to_banned_unknown_and_own_ids_are_dropped(pool: PgPool) {
    let _serial = serial().await;
    let chat = bench(&pool);
    reset(&chat).await;
    slow_mode(&pool, 0).await;
    let (author, banned, outsider) = (user(), user(), user());
    open(&chat, &author).await;
    open(&chat, &banned).await;
    let ban = ask(
        &chat,
        OWNER,
        "PUT",
        &format!("/chat/public/bans/{banned}"),
        Some(json!({})),
    )
    .await;
    assert_eq!(ban.0, 204, "{ban:?}");
    let banned_before = version_of(&chat, &banned).await;
    let sent = send(&chat, &author, pinging(&[&banned, &outsider, &author])).await;
    assert_eq!(sent.0, 200, "{sent:?}");
    assert_eq!(version_of(&chat, &banned).await, banned_before);
    for who in [&outsider, &author] {
        assert_eq!(version_of(&chat, who).await, None, "{who}");
    }
    assert_eq!(ping_seq(&pool, &banned).await, 0);
    assert_eq!(ping_seq(&pool, &author).await, 0);
    let members: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM chat.members WHERE conversation_id = $1 AND user_id = $2",
    )
    .bind(ID)
    .bind(&outsider)
    .fetch_one(&pool)
    .await
    .expect("a count");
    assert_eq!(members, 0);
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn pings_over_the_limit_are_dropped_and_the_post_still_goes(pool: PgPool) {
    let _serial = serial().await;
    let chat = bench(&pool);
    reset(&chat).await;
    slow_mode(&pool, 0).await;
    let (author, target) = (user(), user());
    open(&chat, &author).await;
    open(&chat, &target).await;
    set_counter(&chat, &target, 19).await;
    let first = send(&chat, &author, pinging(&[&target])).await;
    assert_eq!(first.0, 200, "{first:?}");
    let seq = first.1["seq"].as_i64().expect("a seq");
    assert_eq!(ping_seq(&pool, &target).await, seq);
    let second = send(&chat, &author, pinging(&[&target])).await;
    assert_eq!(second.0, 200, "{second:?}");
    assert_eq!(ping_seq(&pool, &target).await, seq);
    assert_eq!(counter(&chat, &target).await, Some(20));
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn the_read_mark_clears_pinged_and_stays_private(pool: PgPool) {
    let _serial = serial().await;
    let chat = bench(&pool);
    reset(&chat).await;
    slow_mode(&pool, 0).await;
    let (author, target) = (user(), user());
    open(&chat, &author).await;
    open(&chat, &target).await;
    let sent = send(&chat, &author, pinging(&[&target])).await;
    let seq = sent.1["seq"].as_i64().expect("a seq");
    assert_eq!(post(&chat, &author).await.0, 200);
    assert!(pinged(&chat, &target).await);

    let early = ask_as(
        &chat,
        with_cap(&target),
        "POST",
        "/chat/sync",
        Some(json!({"pos": 0, "hold_ms": 0, "max_bytes": 262_144, "read": [[ID, seq - 1]]})),
    )
    .await;
    assert_eq!(early.0, 200, "{early:?}");
    assert!(pinged(&chat, &target).await);

    let read = ask_as(
        &chat,
        with_cap(&target),
        "POST",
        "/chat/sync",
        Some(json!({"pos": 0, "hold_ms": 0, "max_bytes": 262_144, "read": [[ID, seq + 1]]})),
    )
    .await;
    assert_eq!(read.0, 200, "{read:?}");
    assert_eq!(read_seq(&pool, &target).await, seq + 1);
    assert!(!pinged(&chat, &target).await);
    let listed = ask(&chat, &target, "GET", "/chat/conversations", None).await;
    let row = listed.1["items"]
        .as_array()
        .expect("items")
        .iter()
        .find(|item| item["id"] == json!(ID))
        .expect("the public row");
    assert_eq!(row["pinged"], json!(false));
    assert_eq!(row["me"]["last_read_seq"], json!(seq + 1));

    let mut conn = chat.redis.get().await.expect("a connection");
    let marks: i64 = redis::cmd("EXISTS")
        .arg(rt_keys::read_marks(ID))
        .query_async(&mut conn)
        .await
        .expect("a probe");
    assert_eq!(marks, 0);
    assert_eq!(read_seq(&pool, &author).await, 0);
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn a_sender_without_pings_still_posts_and_extra_pings_are_refused(pool: PgPool) {
    let _serial = serial().await;
    let chat = bench(&pool);
    reset(&chat).await;
    slow_mode(&pool, 0).await;
    let (author, target) = (user(), user());
    open(&chat, &author).await;
    open(&chat, &target).await;
    let plain = send(
        &chat,
        &author,
        json!({"client_id": Uuid::now_v7(), "class": "post", "epoch": 0, "body": "AQID"}),
    )
    .await;
    assert_eq!(plain.0, 200, "{plain:?}");
    assert_eq!(ping_seq(&pool, &target).await, 0);

    let twice = send(&chat, &author, pinging(&[&target, &target])).await;
    assert_eq!(twice.0, 200, "{twice:?}");
    assert_eq!(counter(&chat, &target).await, Some(1));

    let ids: Vec<String> = (0..6).map(|_| user()).collect();
    let refs: Vec<&str> = ids.iter().map(String::as_str).collect();
    let many = send(&chat, &author, pinging(&refs)).await;
    assert_eq!(error(&many), (400, "chat_invalid"));
}
