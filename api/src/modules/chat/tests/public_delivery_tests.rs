use std::time::Duration;

use sqlx::PgPool;

use crate::modules::chat::media::support::redis;
use crate::modules::chat::moderation::actions;
use crate::modules::chat::moderation::testing::{
    TestKeys, conversation_key, post as stored, state_with,
};
use crate::modules::chat::public_chat::support::{
    OWNER, age_membership, ask, bench, error, moderator, open, posted, reset, serial, slow_mode,
    user, version_of, watch,
};
use crate::modules::chat::public_chat::{ID, delivery, watchers};
use crate::modules::chat::sweeps::public_trail;
use crate::modules::chat::sync::changes::{self, Change};
use crate::modules::chat::{ChatState, rt_keys};

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

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn a_post_wakes_only_the_members_who_watch(pool: PgPool) {
    let _serial = serial().await;
    let chat = bench(&pool);
    reset(&chat).await;
    let (watcher, away, member) = (user(), user(), user());
    for who in [&watcher, &away, &member] {
        open(&chat, who).await;
    }
    watch(&chat, &watcher).await;
    posted(&chat, OWNER).await;
    assert!(version_of(&chat, &watcher).await.is_some());
    assert!(
        deltas_of(&chat, &watcher)
            .await
            .contains(&Change::Conversation(ID))
    );
    assert_eq!(version_of(&chat, &away).await, None);
    assert_eq!(version_of(&chat, &member).await, None);
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn wakes_are_coalesced_to_one_per_two_seconds_with_a_trailing_one(pool: PgPool) {
    let _serial = serial().await;
    let chat = bench(&pool);
    reset(&chat).await;
    let ticker = tokio::spawn(public_trail::run(chat.clone()));
    let watcher = user();
    open(&chat, &watcher).await;
    watch(&chat, &watcher).await;
    posted(&chat, OWNER).await;
    let first = version_of(&chat, &watcher).await.expect("a first wake");
    posted(&chat, OWNER).await;
    posted(&chat, OWNER).await;
    posted(&chat, OWNER).await;
    assert_eq!(version_of(&chat, &watcher).await, Some(first));
    tokio::time::sleep(delivery::WAKE_EVERY + Duration::from_millis(1_000)).await;
    let trailing = version_of(&chat, &watcher).await.expect("a trailing wake");
    assert!(trailing > first);
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(version_of(&chat, &watcher).await, Some(trailing));
    posted(&chat, OWNER).await;
    assert_eq!(version_of(&chat, &watcher).await, Some(trailing));
    tokio::time::sleep(delivery::WAKE_EVERY + Duration::from_millis(1_000)).await;
    assert!(version_of(&chat, &watcher).await > Some(trailing));
    ticker.abort();
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn a_heartbeat_keeps_a_watcher_for_a_minute_and_leaving_drops_it(pool: PgPool) {
    let _serial = serial().await;
    let chat = bench(&pool);
    reset(&chat).await;
    let (one, two) = (user(), user());
    for who in [&one, &two] {
        open(&chat, who).await;
        watch(&chat, who).await;
    }
    watch(&chat, &one).await;
    assert_eq!(watchers::count(&chat).await.expect("a count"), 2);
    let stale = chrono::Utc::now().timestamp_millis() - watchers::TTL_MS - 1_000;
    let mut conn = chat.redis.get().await.expect("a connection");
    let _: i64 = redis::cmd("ZADD")
        .arg(rt_keys::PUBLIC_WATCHERS)
        .arg(stale)
        .arg(&two)
        .query_async(&mut conn)
        .await
        .expect("an old heartbeat");
    drop(conn);
    assert_eq!(
        watchers::current(&chat).await.expect("watchers"),
        std::slice::from_ref(&one)
    );
    let left = ask(&chat, &one, "DELETE", "/chat/public/watch", None).await;
    assert_eq!(left.0, 204, "{left:?}");
    assert!(watchers::current(&chat).await.expect("watchers").is_empty());
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn a_member_who_never_opened_the_chat_cannot_watch_it(pool: PgPool) {
    let _serial = serial().await;
    let chat = bench(&pool);
    reset(&chat).await;
    let stranger = user();
    let answer = ask(&chat, &stranger, "POST", "/chat/public/watch", None).await;
    assert_eq!(error(&answer), (403, "chat_not_member"));
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn a_ban_drops_the_watcher_and_tells_only_them(pool: PgPool) {
    let _serial = serial().await;
    let chat = bench(&pool);
    reset(&chat).await;
    let (target, other) = (user(), user());
    for who in [&target, &other] {
        open(&chat, who).await;
        watch(&chat, who).await;
    }
    let banned = ask(
        &chat,
        OWNER,
        "PUT",
        &format!("/chat/public/bans/{target}"),
        Some(serde_json::json!({})),
    )
    .await;
    assert_eq!(banned.0, 204, "{banned:?}");
    assert_eq!(
        watchers::current(&chat).await.expect("watchers"),
        std::slice::from_ref(&other)
    );
    assert!(version_of(&chat, &target).await.is_some());
    assert_eq!(version_of(&chat, &other).await, None);
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn removed_messages_are_announced_to_watchers_only(pool: PgPool) {
    let _serial = serial().await;
    let keys = TestKeys::default();
    let chat = state_with(pool.clone(), redis(), None);
    reset(&chat).await;
    let ck = conversation_key(&pool, &keys, ID).await;
    let (watcher, member, poster) = (user(), user(), user());
    for who in [&watcher, &member, &poster] {
        sqlx::query("INSERT INTO chat.members (conversation_id, user_id, state, accepted) VALUES ($1, $2, 'active', true)")
            .bind(ID)
            .bind(who)
            .execute(&pool)
            .await
            .expect("a member");
    }
    {
        let mut conn = chat.redis.get().await.expect("a connection");
        let _: i64 = redis::cmd("ZADD")
            .arg(rt_keys::PUBLIC_WATCHERS)
            .arg(chrono::Utc::now().timestamp_millis())
            .arg(&watcher)
            .query_async(&mut conn)
            .await
            .expect("a watcher");
    }
    let seq = stored(&pool, ID, &ck, &poster, "bad words").await;
    let removed = actions::remove_messages(&chat, ID, &poster, &[seq])
        .await
        .expect("removed");
    actions::announce_removed(&chat, ID, &removed).await;
    assert!(
        deltas_of(&chat, &watcher)
            .await
            .contains(&Change::GoneMessages(ID, vec![seq]))
    );
    assert_eq!(version_of(&chat, &member).await, None);
    assert_eq!(version_of(&chat, &poster).await, None);
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn staff_changes_reach_the_helper_and_watchers_in_one_wake(pool: PgPool) {
    let _serial = serial().await;
    let chat = bench(&pool);
    reset(&chat).await;
    slow_mode(&pool, 0).await;
    let (watcher, helper) = (user(), user());
    open(&chat, &watcher).await;
    watch(&chat, &watcher).await;
    age_membership(&pool, &watcher, 60).await;
    moderator(&chat, &helper).await;
    assert!(version_of(&chat, &helper).await.is_some());
    let before = version_of(&chat, &watcher).await;
    let set = ask(
        &chat,
        &helper,
        "PUT",
        "/chat/public/slow-mode",
        Some(serde_json::json!({"seconds": 12})),
    )
    .await;
    assert_eq!(set.0, 204, "{set:?}");
    assert!(version_of(&chat, &watcher).await > before);
}
