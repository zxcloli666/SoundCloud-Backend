use serde_json::{Value, json};
use sqlx::PgPool;

use crate::modules::chat::devices::test_support::fresh_user;
use crate::modules::chat::public_chat::ID;
use crate::modules::chat::public_chat::support::{
    OWNER, ask, bench, closed, error, keys_status, open, post, reset, serial, user,
};
use crate::modules::chat::users::{self, Scope};

async fn members(pool: &PgPool) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM chat.members WHERE conversation_id = $1")
        .bind(ID)
        .fetch_one(pool)
        .await
        .expect("a count")
}

fn listed(answer: &(u16, Value)) -> Vec<&Value> {
    answer.1["items"]
        .as_array()
        .expect("items")
        .iter()
        .collect()
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn the_migration_seeds_one_public_chat_owned_by_the_owner(pool: PgPool) {
    let row: (String, String, Value, i64) = sqlx::query_as(
        "SELECT kind, created_by, settings, (SELECT count(*) FROM chat.conversations WHERE kind = 'public')
         FROM chat.conversations WHERE id = $1",
    )
    .bind(ID)
    .fetch_one(&pool)
    .await
    .expect("the public chat");
    assert_eq!(row.0, "public");
    assert_eq!(row.1, OWNER);
    assert_eq!(row.2, json!({"slow_mode_s": 5}));
    assert_eq!(row.3, 1);
    let owner: (String, String) = sqlx::query_as(
        "SELECT role, state FROM chat.members WHERE conversation_id = $1 AND user_id = $2",
    )
    .bind(ID)
    .bind(OWNER)
    .fetch_one(&pool)
    .await
    .expect("the owner");
    assert_eq!(owner, ("owner".to_owned(), "active".to_owned()));
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn the_first_open_joins_quietly_and_a_second_changes_nothing(pool: PgPool) {
    let _serial = serial().await;
    let chat = bench(&pool);
    reset(&chat).await;
    let me = user();
    let before = members(&pool).await;
    let epoch_before: i32 =
        sqlx::query_scalar("SELECT key_epoch FROM chat.conversations WHERE id = $1")
            .bind(ID)
            .fetch_one(&pool)
            .await
            .expect("an epoch");
    let first = open(&chat, &me).await;
    assert_eq!(first["conversation"]["kind"], "public");
    assert_eq!(first["conversation"]["me"]["role"], "member");
    assert_eq!(first["conversation"]["me"]["state"], "active");
    assert_eq!(first["conversation"]["me"]["notify"], "mentions");
    assert_eq!(first["conversation"]["settings"]["slow_mode_s"], 5);
    assert_eq!(members(&pool).await, before + 1);
    let second = open(&chat, &me).await;
    assert_eq!(second["conversation"]["id"], first["conversation"]["id"]);
    assert_eq!(members(&pool).await, before + 1);
    let (rekey, epoch): (bool, i32) =
        sqlx::query_as("SELECT rekey, key_epoch FROM chat.conversations WHERE id = $1")
            .bind(ID)
            .fetch_one(&pool)
            .await
            .expect("a conversation");
    assert!(!rekey);
    assert_eq!(epoch, epoch_before);
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn the_list_pins_the_chat_first_for_everyone_without_a_member_row(pool: PgPool) {
    let _serial = serial().await;
    let chat = bench(&pool);
    reset(&chat).await;
    let me = user();
    let before = members(&pool).await;
    let page = ask(&chat, &me, "GET", "/chat/conversations", None).await;
    assert_eq!(page.0, 200, "{page:?}");
    let items = listed(&page);
    assert_eq!(items[0]["kind"], "public");
    assert_eq!(items[0]["id"], json!(ID));
    assert!(items[0]["me"]["pinned_at"].is_i64());
    assert_eq!(members(&pool).await, before);
    let next = ask(
        &chat,
        &me,
        "GET",
        &format!("/chat/conversations?cursor={}", uuid::Uuid::nil()),
        None,
    )
    .await;
    assert!(listed(&next).iter().all(|item| item["kind"] != "public"));
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn a_member_sees_the_latest_seq_in_the_list_without_being_woken(pool: PgPool) {
    let _serial = serial().await;
    let chat = bench(&pool);
    reset(&chat).await;
    let (reader, writer) = (user(), user());
    open(&chat, &reader).await;
    open(&chat, &writer).await;
    let sent = ask(
        &chat,
        OWNER,
        "POST",
        &format!("/chat/conversations/{ID}/messages"),
        Some(crate::modules::chat::public_chat::support::post_body(&[])),
    )
    .await;
    assert_eq!(sent.0, 200, "{sent:?}");
    let page = ask(&chat, &reader, "GET", "/chat/conversations", None).await;
    assert_eq!(listed(&page)[0]["last_seq"], sent.1["seq"]);
    assert_eq!(
        crate::modules::chat::public_chat::support::version_of(&chat, &reader).await,
        None
    );
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn the_kill_switch_hides_the_chat_and_refuses_every_public_route(pool: PgPool) {
    let _serial = serial().await;
    let chat = closed(&pool);
    reset(&chat).await;
    let me = user();
    let page = ask(&chat, &me, "GET", "/chat/conversations", None).await;
    assert_eq!(page.0, 200);
    assert!(listed(&page).iter().all(|item| item["kind"] != "public"));
    let disabled = (503, "chat_disabled");
    for (method, uri, body) in [
        ("POST", "/chat/public/open".to_owned(), None),
        ("POST", "/chat/public/watch".to_owned(), None),
        ("GET", "/chat/public/log".to_owned(), None),
        (
            "PUT",
            "/chat/public/slow-mode".to_owned(),
            Some(json!({"seconds": 3})),
        ),
        ("GET", format!("/chat/conversations/{ID}"), None),
        ("GET", format!("/chat/conversations/{ID}/messages"), None),
        ("GET", format!("/chat/conversations/{ID}/members"), None),
    ] {
        let answer = ask(&chat, OWNER, method, &uri, body).await;
        assert_eq!(error(&answer), disabled, "{method} {uri}");
    }
    let sent = post(&chat, OWNER).await;
    assert_eq!(error(&sent), disabled);
    let keys = keys_status(&chat, &pool, &fresh_user()).await;
    assert_eq!(error(&keys), disabled);
    let own = ask(&chat, &me, "GET", "/chat/conversations", None).await;
    assert_eq!(own.0, 200, "other chats keep working");
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn a_restriction_with_scope_all_stops_opening_reading_and_posting(pool: PgPool) {
    let _serial = serial().await;
    let chat = bench(&pool);
    reset(&chat).await;
    let me = user();
    open(&chat, &me).await;
    let until = chrono::Utc::now() + chrono::Duration::days(1);
    users::restrict(&chat, &me, until, &[Scope::All])
        .await
        .expect("a restriction");
    let refused = ("chat_muted", 403);
    let opened = ask(&chat, &me, "POST", "/chat/public/open", None).await;
    assert_eq!((error(&opened).1, opened.0), refused);
    assert_eq!(opened.1["until"], until.timestamp_millis());
    let read = ask(
        &chat,
        &me,
        "GET",
        &format!("/chat/conversations/{ID}/messages"),
        None,
    )
    .await;
    assert_eq!((error(&read).1, read.0), refused);
    let sent = post(&chat, &me).await;
    assert_eq!((error(&sent).1, sent.0), refused);
}
