use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

use crate::modules::chat::conversations::fixtures::group;
use crate::modules::chat::media::rows::{Seed, add_ref, seed};
use crate::modules::chat::media::support::{Rig, StoreSpec, bench, bench_with, call, error, user};
use crate::modules::chat::model::MemberState;

fn states(answer: &Value) -> Vec<(String, Option<String>)> {
    answer["items"]
        .as_array()
        .expect("items")
        .iter()
        .map(|item| {
            (
                item["state"].as_str().unwrap_or_default().to_owned(),
                item["url"].as_str().map(str::to_owned),
            )
        })
        .collect()
}

fn kinds(answer: &Value) -> Vec<String> {
    states(answer).into_iter().map(|(state, _)| state).collect()
}

async fn set_floor(pool: &PgPool, conversation: Uuid, member: &str, joined: i64) {
    sqlx::query(
        "UPDATE chat.members SET joined_seq = $3 WHERE conversation_id = $1 AND user_id = $2",
    )
    .bind(conversation)
    .bind(member)
    .bind(joined)
    .execute(pool)
    .await
    .expect("a floor");
    sqlx::query(
        "UPDATE chat.conversations SET settings = '{\"history_visible\": false}' WHERE id = $1",
    )
    .bind(conversation)
    .execute(pool)
    .await
    .expect("hidden history");
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn members_read_above_their_floor_and_everyone_else_sees_deleted(pool: PgPool) {
    let chat = bench(&pool);
    let (owner, late, stranger) = (user(), user(), user());
    let group = group(&pool, &owner, &[(&late, MemberState::Active)]).await;
    set_floor(&pool, group, &late, 5).await;
    let (old, old_thumb) = seed(&pool, Seed::image(&owner, group, "attached")).await;
    let (new, new_thumb) = seed(&pool, Seed::image(&owner, group, "attached")).await;
    let (avatar, _) = seed(
        &pool,
        Seed {
            kind: "avatar",
            ..Seed::image(&owner, group, "attached")
        },
    )
    .await;
    add_ref(&pool, old, group, 3).await;
    add_ref(&pool, new, group, 9).await;
    add_ref(&pool, avatar, group, 0).await;
    let ids = vec![
        new,
        old,
        new_thumb.unwrap(),
        old_thumb.unwrap(),
        avatar,
        Uuid::new_v4(),
    ];
    let body = json!({"conversation_id": group, "ids": ids});
    let answer = call(&chat, &late, "POST", "/chat/media/urls", Some(body.clone())).await;
    assert_eq!(answer.0, 200, "{answer:?}");
    assert_eq!(answer.1["expires_in_s"], 900);
    assert_eq!(
        kinds(&answer.1),
        ["ok", "deleted", "ok", "deleted", "ok", "deleted"]
    );
    let items = answer.1["items"].as_array().expect("items");
    for (item, id) in items.iter().zip(&ids) {
        assert_eq!(item["id"], json!(id));
    }
    let url = items[0]["url"].as_str().expect("a url");
    assert!(
        url.starts_with("https://s3.scnative.dev/scd-chat/m/") && url.contains("X-Amz-Expires=900"),
        "{url}"
    );
    let own = call(
        &chat,
        &owner,
        "POST",
        "/chat/media/urls",
        Some(json!({"ids": ids})),
    )
    .await;
    assert_eq!(kinds(&own.1), ["ok", "ok", "ok", "ok", "ok", "deleted"]);
    let member_without = call(
        &chat,
        &late,
        "POST",
        "/chat/media/urls",
        Some(json!({"ids": [new]})),
    )
    .await;
    assert_eq!(kinds(&member_without.1), ["deleted"]);
    let outsider = call(&chat, &stranger, "POST", "/chat/media/urls", Some(body)).await;
    assert_eq!(error(&outsider), (403, "chat_not_member"));
    let outsider = call(
        &chat,
        &stranger,
        "POST",
        "/chat/media/urls",
        Some(json!({"ids": [new, old]})),
    )
    .await;
    assert_eq!(kinds(&outsider.1), ["deleted", "deleted"]);
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn lost_reserved_held_and_deleting_rows_answer_their_state(pool: PgPool) {
    let chat = bench(&pool);
    let (owner, member) = (user(), user());
    let group = group(&pool, &owner, &[(&member, MemberState::Active)]).await;
    let mut ids = Vec::new();
    for state in ["lost", "reserved", "held", "deleting", "uploaded"] {
        let (root, _) = seed(&pool, Seed::image(&owner, group, state)).await;
        add_ref(&pool, root, group, 4).await;
        ids.push(root);
    }
    let body = json!({"conversation_id": group, "ids": ids});
    let member_view = call(
        &chat,
        &member,
        "POST",
        "/chat/media/urls",
        Some(body.clone()),
    )
    .await;
    assert_eq!(
        kinds(&member_view.1),
        ["lost", "deleted", "deleted", "deleted", "ok"]
    );
    let owner_view = call(
        &chat,
        &owner,
        "POST",
        "/chat/media/urls",
        Some(json!({"ids": ids})),
    )
    .await;
    assert_eq!(
        kinds(&owner_view.1),
        ["lost", "deleted", "deleted", "deleted", "ok"]
    );
    assert!(states(&owner_view.1)[0].1.is_none());
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn reads_sign_on_the_store_of_each_row(pool: PgPool) {
    let mut rig = Rig::offline();
    rig.stores.push(StoreSpec {
        id: 2,
        bucket: "scd-chat-2".to_owned(),
        cap_gb: 200,
    });
    rig.write = 2;
    let chat = bench_with(&pool, &rig);
    let owner = user();
    let group = group(&pool, &owner, &[]).await;
    let (first, _) = seed(&pool, Seed::image(&owner, group, "uploaded")).await;
    let (second, _) = seed(
        &pool,
        Seed {
            store: 2,
            ..Seed::image(&owner, group, "uploaded")
        },
    )
    .await;
    let answer = call(
        &chat,
        &owner,
        "POST",
        "/chat/media/urls",
        Some(json!({"ids": [first, second]})),
    )
    .await;
    let urls: Vec<String> = states(&answer.1)
        .into_iter()
        .filter_map(|(_, url)| url)
        .collect();
    assert!(
        urls[0].starts_with("https://s3.scnative.dev/scd-chat/m/"),
        "{}",
        urls[0]
    );
    assert!(
        urls[1].starts_with("https://s3.scnative.dev/scd-chat-2/m/"),
        "{}",
        urls[1]
    );
    let keys: Vec<String> =
        sqlx::query_scalar("SELECT object_key FROM chat.media WHERE id = ANY($1) ORDER BY store")
            .bind(vec![first, second])
            .fetch_all(&pool)
            .await
            .expect("keys");
    assert!(urls[0].contains(&keys[0]) && urls[1].contains(&keys[1]));
}
