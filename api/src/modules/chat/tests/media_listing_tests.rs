use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

use crate::modules::chat::conversations::fixtures::group;
use crate::modules::chat::media::rows::{Seed, add_ref, seed};
use crate::modules::chat::media::support::{AVATAR, IMAGE, THUMB, bench, call, error, user};
use crate::modules::chat::model::MemberState;

type RateCase = (
    &'static str,
    &'static str,
    Option<Value>,
    usize,
    (u16, &'static str),
);

const TOTAL: i64 = (IMAGE + THUMB) as i64;

async fn age(pool: &PgPool, id: Uuid, minutes: i32, bytes: Option<i64>) {
    sqlx::query(
        "UPDATE chat.media SET created_at = now() - $2::int * interval '1 minute', bytes = coalesce($3, bytes) WHERE id = $1",
    )
    .bind(id)
    .bind(minutes)
    .bind(bytes)
    .execute(pool)
    .await
    .expect("an aged row");
}

fn ids(page: &Value) -> Vec<Uuid> {
    page["items"]
        .as_array()
        .expect("items")
        .iter()
        .map(|item| serde_json::from_value(item["id"].clone()).expect("an id"))
        .collect()
}

async fn walk(
    chat: &std::sync::Arc<crate::modules::chat::ChatState>,
    owner: &str,
    query: &str,
) -> Vec<Uuid> {
    let mut seen = Vec::new();
    let mut cursor: Option<String> = None;
    loop {
        let uri = match &cursor {
            Some(cursor) => format!("/chat/media/mine?{query}&limit=2&cursor={cursor}"),
            None => format!("/chat/media/mine?{query}&limit=2"),
        };
        let page = call(chat, owner, "GET", &uri, None).await;
        assert_eq!(page.0, 200, "{page:?}");
        seen.extend(ids(&page.1));
        match page.1["next_cursor"].as_str() {
            Some(next) => cursor = Some(next.to_owned()),
            None => return seen,
        }
    }
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn mine_lists_own_live_rows_in_three_orders_with_an_opaque_cursor(pool: PgPool) {
    let chat = bench(&pool);
    let (owner, other) = (user(), user());
    let first = group(&pool, &owner, &[]).await;
    let second = group(&pool, &owner, &[]).await;
    let (a, a_thumb) = seed(&pool, Seed::image(&owner, first, "attached")).await;
    let (b, _) = seed(&pool, Seed::image(&owner, first, "uploaded")).await;
    let (c, _) = seed(&pool, Seed::image(&owner, second, "lost")).await;
    let (d, _) = seed(
        &pool,
        Seed {
            kind: "avatar",
            ..Seed::image(&owner, second, "attached")
        },
    )
    .await;
    let (e, _) = seed(&pool, Seed::image(&owner, first, "uploaded")).await;
    seed(&pool, Seed::image(&owner, first, "reserved")).await;
    seed(&pool, Seed::image(&owner, first, "deleting")).await;
    seed(&pool, Seed::image(&other, first, "uploaded")).await;
    for (id, minutes, bytes) in [
        (a, 50, Some(8192)),
        (b, 40, None),
        (c, 30, Some(4096)),
        (d, 20, None),
        (e, 10, Some(614_400)),
    ] {
        age(&pool, id, minutes, bytes).await;
    }
    add_ref(&pool, a, first, 1).await;
    add_ref(&pool, a, second, 1).await;
    add_ref(&pool, a, second, 2).await;
    assert_eq!(walk(&chat, &owner, "order=new").await, [e, d, c, b, a]);
    assert_eq!(walk(&chat, &owner, "order=old").await, [a, b, c, d, e]);
    assert_eq!(walk(&chat, &owner, "order=big").await, [e, b, a, c, d]);
    assert_eq!(
        walk(&chat, &owner, &format!("conversation_id={second}")).await,
        [d, c]
    );
    let page = call(
        &chat,
        &owner,
        "GET",
        "/chat/media/mine?order=old&limit=1",
        None,
    )
    .await;
    let item = &page.1["items"][0];
    assert_eq!(item["thumb_id"], json!(a_thumb));
    assert_eq!(
        (item["bytes"].as_i64(), item["chats"].as_i64()),
        (Some(8192 + THUMB as i64), Some(2))
    );
    assert_eq!(
        (item["kind"].as_str(), item["status"].as_str()),
        (Some("image"), Some("attached"))
    );
    assert_eq!(item["conversation_id"], json!(first));
    let avatar = &call(
        &chat,
        &owner,
        "GET",
        &format!("/chat/media/mine?conversation_id={second}"),
        None,
    )
    .await
    .1["items"][0];
    assert_eq!(
        (avatar["kind"].as_str(), avatar["thumb_id"].clone()),
        (Some("avatar"), Value::Null)
    );
    assert_eq!(avatar["bytes"].as_i64(), Some(AVATAR as i64));
    let cursor = page.1["next_cursor"].as_str().expect("a cursor").to_owned();
    let wrong = call(
        &chat,
        &owner,
        "GET",
        &format!("/chat/media/mine?order=big&cursor={cursor}"),
        None,
    )
    .await;
    assert_eq!(error(&wrong), (400, "chat_invalid"));
    let junk = call(
        &chat,
        &owner,
        "GET",
        "/chat/media/mine?cursor=not-a-cursor",
        None,
    )
    .await;
    assert_eq!(error(&junk), (400, "chat_invalid"));
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn usage_groups_own_rows_by_origin_and_marks_gone_conversations(pool: PgPool) {
    let chat = bench(&pool);
    let (owner, peer) = (user(), user());
    let kept = group(&pool, &owner, &[]).await;
    let left = group(&pool, &peer, &[(&owner, MemberState::Active)]).await;
    let deleted = group(&pool, &owner, &[]).await;
    for _ in 0..2 {
        seed(&pool, Seed::image(&owner, kept, "attached")).await;
    }
    seed(&pool, Seed::image(&owner, left, "uploaded")).await;
    seed(
        &pool,
        Seed {
            kind: "avatar",
            ..Seed::image(&owner, deleted, "attached")
        },
    )
    .await;
    seed(&pool, Seed::image(&owner, kept, "reserved")).await;
    seed(&pool, Seed::image(&owner, kept, "lost")).await;
    sqlx::query(
        "UPDATE chat.members SET state = 'left' WHERE conversation_id = $1 AND user_id = $2",
    )
    .bind(left)
    .bind(&owner)
    .execute(&pool)
    .await
    .expect("a leave");
    sqlx::query("UPDATE chat.conversations SET deleted_at = now() WHERE id = $1")
        .bind(deleted)
        .execute(&pool)
        .await
        .expect("a delete");
    let answer = call(&chat, &owner, "GET", "/chat/media/usage", None).await;
    assert_eq!(answer.0, 200, "{answer:?}");
    assert_eq!(answer.1["bytes"].as_i64(), Some(3 * TOTAL + AVATAR as i64));
    assert_eq!(
        (answer.1["reserved"].as_i64(), answer.1["files"].as_i64()),
        (Some(TOTAL), Some(4))
    );
    assert_eq!(
        answer.1["conversations"],
        json!([
            {"conversation_id": kept, "bytes": 2 * TOTAL, "files": 2, "gone": false},
            {"conversation_id": left, "bytes": TOTAL, "files": 1, "gone": true},
            {"conversation_id": deleted, "bytes": AVATAR, "files": 1, "gone": true},
        ])
    );
    let empty = call(&chat, &user(), "GET", "/chat/media/usage", None).await;
    assert_eq!(empty.1["conversations"], json!([]));
    assert_eq!(empty.1["bytes"], 0);
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn the_read_and_write_routes_keep_their_rates(pool: PgPool) {
    let chat = bench(&pool);
    let owner = user();
    let cases: [RateCase; 4] = [
        (
            "GET",
            "/chat/media/usage",
            None,
            60,
            (429, "chat_read_limited"),
        ),
        (
            "GET",
            "/chat/media/mine",
            None,
            60,
            (429, "chat_read_limited"),
        ),
        (
            "POST",
            "/chat/media/delete",
            Some(json!({"all": true})),
            30,
            (429, "chat_request_limit"),
        ),
        (
            "POST",
            "/chat/media/restore",
            Some(json!({"all": true})),
            30,
            (429, "chat_request_limit"),
        ),
    ];
    for (method, uri, body, allowed, refused) in cases {
        for _ in 0..allowed {
            let answer = call(&chat, &owner, method, uri, body.clone()).await;
            assert_eq!(answer.0, 200, "{uri} {answer:?}");
        }
        let answer = call(&chat, &owner, method, uri, body).await;
        assert_eq!(error(&answer), refused, "{uri}");
    }
}
