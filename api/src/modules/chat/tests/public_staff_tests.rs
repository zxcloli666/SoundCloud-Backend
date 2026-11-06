use serde_json::{Value, json};
use sqlx::PgPool;

use crate::modules::chat::public_chat::ID;
use crate::modules::chat::public_chat::support::{
    OWNER, age_membership, ask, audit_count, bench, error, moderator, open, posted, reset, serial,
    slow_mode, user,
};

fn path(kind: &str, user: &str) -> String {
    format!("/chat/public/{kind}/{user}")
}

async fn role_of(pool: &PgPool, user: &str) -> (String, String) {
    sqlx::query_as(
        "SELECT role, state FROM chat.members WHERE conversation_id = $1 AND user_id = $2",
    )
    .bind(ID)
    .bind(user)
    .fetch_one(pool)
    .await
    .expect("a member")
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn only_the_owner_adds_and_removes_moderators(pool: PgPool) {
    let _serial = serial().await;
    let chat = bench(&pool);
    reset(&chat).await;
    let (first, second, plain) = (user(), user(), user());
    open(&chat, &plain).await;
    moderator(&chat, &first).await;
    assert_eq!(role_of(&pool, &first).await.0, "admin");
    let denied = (403, "chat_not_member");
    for caller in [&first, &plain] {
        let answer = ask(
            &chat,
            caller,
            "PUT",
            &path("moderators", &second),
            Some(json!({"on": true})),
        )
        .await;
        assert_eq!(error(&answer), denied);
    }
    let staff = ask(
        &chat,
        &plain,
        "GET",
        &format!("/chat/conversations/{ID}/members"),
        None,
    )
    .await;
    let names: Vec<&str> = staff
        .1
        .as_array()
        .expect("staff")
        .iter()
        .map(|row| row["user_id"].as_str().unwrap())
        .collect();
    assert!(names.contains(&OWNER) && names.contains(&first.as_str()));
    assert!(!names.contains(&plain.as_str()));
    let removed = ask(
        &chat,
        OWNER,
        "PUT",
        &path("moderators", &first),
        Some(json!({"on": false})),
    )
    .await;
    assert_eq!(removed.0, 204, "{removed:?}");
    assert_eq!(role_of(&pool, &first).await.0, "member");
    let owner_self = ask(
        &chat,
        OWNER,
        "PUT",
        &path("moderators", OWNER),
        Some(json!({"on": false})),
    )
    .await;
    assert_eq!(error(&owner_self), (400, "chat_invalid"));
    assert_eq!(audit_count(&pool, "moderator_add").await, 1);
    assert_eq!(audit_count(&pool, "moderator_remove").await, 1);
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn a_moderator_cannot_touch_the_owner_or_another_moderator(pool: PgPool) {
    let _serial = serial().await;
    let chat = bench(&pool);
    reset(&chat).await;
    let (one, two, plain) = (user(), user(), user());
    moderator(&chat, &one).await;
    moderator(&chat, &two).await;
    open(&chat, &plain).await;
    let denied = (403, "chat_not_member");
    for target in [OWNER, two.as_str(), one.as_str()] {
        let muted = ask(
            &chat,
            &one,
            "PUT",
            &path("mutes", target),
            Some(json!({"seconds": 600})),
        )
        .await;
        assert_eq!(error(&muted), denied, "mute {target}");
        let banned = ask(&chat, &one, "PUT", &path("bans", target), Some(json!({}))).await;
        assert_eq!(error(&banned), denied, "ban {target}");
    }
    let owner_hit = ask(&chat, OWNER, "PUT", &path("bans", OWNER), Some(json!({}))).await;
    assert_eq!(error(&owner_hit), denied);
    let allowed = ask(
        &chat,
        &one,
        "PUT",
        &path("mutes", &plain),
        Some(json!({"seconds": 600})),
    )
    .await;
    assert_eq!(allowed.0, 200, "{allowed:?}");
    let banned = ask(&chat, &one, "PUT", &path("bans", &plain), Some(json!({}))).await;
    assert_eq!(banned.0, 204, "{banned:?}");
    let by_member = ask(
        &chat,
        &plain,
        "PUT",
        &path("mutes", &one),
        Some(json!({"seconds": 600})),
    )
    .await;
    assert_eq!(error(&by_member), denied);
    let owner_bans_mod = ask(&chat, OWNER, "PUT", &path("bans", &two), Some(json!({}))).await;
    assert_eq!(owner_bans_mod.0, 204, "{owner_bans_mod:?}");
    assert_eq!(
        role_of(&pool, &two).await,
        ("member".to_owned(), "banned".to_owned())
    );
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn every_action_leaves_an_audit_row_and_the_log_follows_the_role(pool: PgPool) {
    let _serial = serial().await;
    let chat = bench(&pool);
    reset(&chat).await;
    let (helper, plain) = (user(), user());
    moderator(&chat, &helper).await;
    open(&chat, &plain).await;
    age_membership(&pool, &plain, 60).await;
    let seq = posted(&chat, &plain).await;
    let actions = [
        (
            "PUT",
            path("mutes", &plain),
            Some(json!({"seconds": 600})),
            200,
        ),
        ("DELETE", path("mutes", &plain), None, 204),
        (
            "PUT",
            path("bans", &plain),
            Some(json!({"reason": "x"})),
            204,
        ),
        ("DELETE", path("bans", &plain), None, 204),
        (
            "PUT",
            "/chat/public/slow-mode".to_owned(),
            Some(json!({"seconds": 9})),
            204,
        ),
    ];
    for (method, uri, body, status) in actions {
        let answer = ask(&chat, OWNER, method, &uri, body).await;
        assert_eq!(answer.0, status, "{method} {uri} {answer:?}");
    }
    let deleted = ask(
        &chat,
        &helper,
        "DELETE",
        &format!("/chat/conversations/{ID}/messages/{seq}?for=everyone"),
        None,
    )
    .await;
    assert_eq!(deleted.0, 204, "{deleted:?}");
    for action in [
        "mute",
        "unmute",
        "ban",
        "unban",
        "slow_mode",
        "delete",
        "moderator_add",
    ] {
        assert_eq!(audit_count(&pool, action).await, 1, "{action}");
    }
    let all = ask(&chat, OWNER, "GET", "/chat/public/log", None).await;
    assert_eq!(all.0, 200, "{all:?}");
    let entries = all.1["entries"].as_array().expect("entries");
    assert_eq!(entries.len(), 7);
    assert_eq!(entries[0]["action"], "delete");
    assert_eq!(entries[0]["by"], json!(helper));
    assert_eq!(entries[0]["detail"]["author"], json!(plain));
    let own = ask(&chat, &helper, "GET", "/chat/public/log", None).await;
    let own_entries = own.1["entries"].as_array().expect("entries");
    assert_eq!(own_entries.len(), 1);
    assert_eq!(own_entries[0]["action"], "delete");
    let denied = ask(&chat, &plain, "GET", "/chat/public/log", None).await;
    assert_eq!(error(&denied), (403, "chat_not_member"));
    let page = ask(&chat, OWNER, "GET", "/chat/public/log?limit=3", None).await;
    assert_eq!(page.1["entries"].as_array().unwrap().len(), 3);
    let next = page.1["next"].as_str().expect("a cursor");
    let rest = ask(
        &chat,
        OWNER,
        "GET",
        &format!("/chat/public/log?limit=50&before={next}"),
        None,
    )
    .await;
    assert_eq!(rest.1["entries"].as_array().unwrap().len(), 4);
    assert_eq!(rest.1["next"], Value::Null);
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn staff_delete_any_message_and_members_only_their_own(pool: PgPool) {
    let _serial = serial().await;
    let chat = bench(&pool);
    reset(&chat).await;
    slow_mode(&pool, 0).await;
    let (author, bystander, helper) = (user(), user(), user());
    for who in [&author, &bystander] {
        open(&chat, who).await;
        age_membership(&pool, who, 60).await;
    }
    moderator(&chat, &helper).await;
    let uri = |seq: i64| format!("/chat/conversations/{ID}/messages/{seq}?for=everyone");
    let first = posted(&chat, &author).await;
    let denied = ask(&chat, &bystander, "DELETE", &uri(first), None).await;
    assert_eq!(error(&denied), (403, "chat_not_member"));
    let by_helper = ask(&chat, &helper, "DELETE", &uri(first), None).await;
    assert_eq!(by_helper.0, 204, "{by_helper:?}");
    let second = posted(&chat, &author).await;
    let own = ask(&chat, &author, "DELETE", &uri(second), None).await;
    assert_eq!(own.0, 204, "{own:?}");
    let third = posted(&chat, &author).await;
    let by_owner = ask(&chat, OWNER, "DELETE", &uri(third), None).await;
    assert_eq!(by_owner.0, 204, "{by_owner:?}");
    assert_eq!(audit_count(&pool, "delete").await, 2);
    let body: Option<Vec<u8>> = sqlx::query_scalar(
        "SELECT body FROM chat.messages WHERE conversation_id = $1 AND seq = $2",
    )
    .bind(ID)
    .bind(first)
    .fetch_one(&pool)
    .await
    .expect("a row");
    assert!(body.is_none());
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn slow_mode_is_set_by_staff_within_range_and_shows_in_the_view(pool: PgPool) {
    let _serial = serial().await;
    let chat = bench(&pool);
    reset(&chat).await;
    let (helper, plain) = (user(), user());
    moderator(&chat, &helper).await;
    open(&chat, &plain).await;
    let set = |who: String, seconds: u32| {
        let chat = chat.clone();
        async move {
            ask(
                &chat,
                &who,
                "PUT",
                "/chat/public/slow-mode",
                Some(json!({"seconds": seconds})),
            )
            .await
        }
    };
    assert_eq!(
        error(&set(plain.clone(), 3).await),
        (403, "chat_not_member")
    );
    assert_eq!(
        error(&set(helper.clone(), 601).await),
        (400, "chat_invalid")
    );
    assert_eq!(set(helper.clone(), 30).await.0, 204);
    let view = open(&chat, &plain).await;
    assert_eq!(view["conversation"]["settings"]["slow_mode_s"], 30);
    assert_eq!(set(OWNER.to_owned(), 0).await.0, 204);
    let view = open(&chat, &plain).await;
    assert!(
        view["conversation"]["settings"]
            .get("slow_mode_s")
            .is_none()
    );
}
