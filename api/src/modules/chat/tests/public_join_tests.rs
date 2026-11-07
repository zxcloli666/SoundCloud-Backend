use serde_json::{Value, json};
use sqlx::PgPool;

use crate::modules::chat::devices::test_support::fresh_user;
use crate::modules::chat::limits::ROOM_NEWCOMER_PLAINTEXT_MAX;
use crate::modules::chat::public_chat::support::{
    OWNER, age_membership, ask, bench, closed, error, keys_status, moderator, open, post, reset,
    serial, user,
};
use crate::modules::chat::public_chat::{ID, view};
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

async fn joined_at_ms(pool: &PgPool, user: &str) -> i64 {
    let at: chrono::DateTime<chrono::Utc> = sqlx::query_scalar(
        "SELECT joined_at FROM chat.members WHERE conversation_id = $1 AND user_id = $2",
    )
    .bind(ID)
    .bind(user)
    .fetch_one(pool)
    .await
    .expect("a join time");
    at.timestamp_millis()
}

fn keys(value: &Value) -> Vec<&str> {
    value
        .as_object()
        .expect("an object")
        .keys()
        .map(String::as_str)
        .collect()
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn the_open_answer_carries_the_mute_and_the_newcomer_window_by_name(pool: PgPool) {
    let _serial = serial().await;
    let chat = bench(&pool);
    reset(&chat).await;
    let me = user();
    let seen = open(&chat, &me).await;
    assert_eq!(keys(&seen), ["conversation", "mute", "newcomer", "online"]);
    assert_eq!(seen["mute"], Value::Null);
    assert_eq!(keys(&seen["newcomer"]), ["body_max", "until"]);
    assert_eq!(
        seen["newcomer"]["until"],
        joined_at_ms(&pool, &me).await + 600_000
    );
    assert_eq!(seen["newcomer"]["body_max"], ROOM_NEWCOMER_PLAINTEXT_MAX);
    assert_eq!(seen["newcomer"]["body_max"], 512);
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn a_muted_member_opens_with_the_mute_and_an_unmute_clears_it(pool: PgPool) {
    let _serial = serial().await;
    let chat = bench(&pool);
    reset(&chat).await;
    let me = user();
    open(&chat, &me).await;
    let muted = ask(
        &chat,
        OWNER,
        "PUT",
        &format!("/chat/public/mutes/{me}"),
        Some(json!({"seconds": 3_600, "reason": "spam"})),
    )
    .await;
    assert_eq!(muted.0, 200, "{muted:?}");
    let seen = open(&chat, &me).await;
    assert_eq!(keys(&seen["mute"]), ["reason", "until"]);
    assert_eq!(seen["mute"]["until"], muted.1["until"]);
    assert_eq!(seen["mute"]["reason"], "spam");
    assert_eq!(seen["conversation"]["me"]["muted_until"], Value::Null);
    let lifted = ask(
        &chat,
        OWNER,
        "DELETE",
        &format!("/chat/public/mutes/{me}"),
        None,
    )
    .await;
    assert_eq!(lifted.0, 204, "{lifted:?}");
    assert_eq!(open(&chat, &me).await["mute"], Value::Null);
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn a_mute_without_a_reason_and_an_expired_one(pool: PgPool) {
    let _serial = serial().await;
    let chat = bench(&pool);
    reset(&chat).await;
    let me = user();
    let muted = ask(
        &chat,
        OWNER,
        "PUT",
        &format!("/chat/public/mutes/{me}"),
        Some(json!({"seconds": 600})),
    )
    .await;
    assert_eq!(muted.0, 200, "{muted:?}");
    let seen = open(&chat, &me).await;
    assert_eq!(seen["mute"]["until"], muted.1["until"]);
    assert_eq!(seen["mute"]["reason"], Value::Null);
    sqlx::query(
        "UPDATE chat.public_mutes SET until = now() - interval '1 second' WHERE user_id = $1",
    )
    .bind(&me)
    .execute(&pool)
    .await
    .expect("an expired mute");
    assert_eq!(open(&chat, &me).await["mute"], Value::Null);
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn the_newcomer_window_has_no_end_before_the_first_join(pool: PgPool) {
    let _serial = serial().await;
    let chat = bench(&pool);
    reset(&chat).await;
    let stranger = user();
    let row = view::row(&chat, &stranger).await.expect("a row");
    assert!(!row.joined());
    let seen = serde_json::to_value(view::opened(&chat, row).await.expect("a view")).unwrap();
    assert_eq!(seen["newcomer"], json!({"until": null, "body_max": 512}));
    assert_eq!(seen["mute"], Value::Null);
    let unstamped = user();
    open(&chat, &unstamped).await;
    sqlx::query(
        "UPDATE chat.members SET joined_at = NULL WHERE conversation_id = $1 AND user_id = $2",
    )
    .bind(ID)
    .bind(&unstamped)
    .execute(&pool)
    .await
    .expect("a member without a join time");
    assert_eq!(
        open(&chat, &unstamped).await["newcomer"],
        json!({"until": null, "body_max": 512})
    );
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn old_members_and_staff_are_not_newcomers(pool: PgPool) {
    let _serial = serial().await;
    let chat = bench(&pool);
    reset(&chat).await;
    let (old, helper) = (user(), user());
    open(&chat, &old).await;
    age_membership(&pool, &old, 11).await;
    assert_eq!(open(&chat, &old).await["newcomer"], Value::Null);
    moderator(&chat, &helper).await;
    let fresh = open(&chat, &helper).await;
    assert_eq!(fresh["conversation"]["me"]["role"], "admin");
    assert_eq!(fresh["newcomer"], Value::Null);
    assert_eq!(open(&chat, OWNER).await["newcomer"], Value::Null);
}
