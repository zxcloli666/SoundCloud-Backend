use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

use crate::modules::chat::conversations::fixtures::group;
use crate::modules::chat::media::limits::QUOTA_BYTES;
use crate::modules::chat::media::rows::{Seed, add_ref, seed, status, usage};
use crate::modules::chat::media::support::{IMAGE, THUMB, bench, call, error, user};

const TOTAL: i64 = (IMAGE + THUMB) as i64;

async fn statuses(pool: &PgPool, root: Uuid) -> Vec<(String, Option<bool>)> {
    sqlx::query_as(
        "SELECT status, restorable_until > now() FROM chat.media WHERE id = $1 OR parent_id = $1 ORDER BY kind",
    )
    .bind(root)
    .fetch_all(pool)
    .await
    .expect("statuses")
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn a_delete_frees_the_quota_at_once_and_a_thumb_id_counts_as_its_image(pool: PgPool) {
    let chat = bench(&pool);
    let (owner, other) = (user(), user());
    let group = group(&pool, &owner, &[]).await;
    let (attached, attached_thumb) = seed(&pool, Seed::image(&owner, group, "attached")).await;
    add_ref(&pool, attached, group, 2).await;
    let (uploaded, _) = seed(&pool, Seed::image(&owner, group, "uploaded")).await;
    let (reserved, _) = seed(&pool, Seed::image(&owner, group, "reserved")).await;
    let (lost, _) = seed(&pool, Seed::image(&owner, group, "lost")).await;
    let (foreign, _) = seed(&pool, Seed::image(&other, group, "uploaded")).await;
    assert_eq!(usage(&pool, &owner).await, (2 * TOTAL, TOTAL, 2));
    let ids = json!([attached_thumb.unwrap(), attached, foreign, Uuid::new_v4()]);
    let answer = call(
        &chat,
        &owner,
        "POST",
        "/chat/media/delete",
        Some(json!({"ids": ids})),
    )
    .await;
    assert_eq!(answer.0, 200, "{answer:?}");
    assert_eq!(
        answer.1,
        json!({"deleted": 1, "freed": TOTAL, "used": 2 * TOTAL, "quota": QUOTA_BYTES})
    );
    assert_eq!(
        statuses(&pool, attached).await,
        vec![("deleting".to_owned(), Some(true)); 2]
    );
    assert_eq!(status(&pool, foreign).await.as_deref(), Some("uploaded"));
    let urls = call(
        &chat,
        &owner,
        "POST",
        "/chat/media/urls",
        Some(json!({"ids": [attached]})),
    )
    .await;
    assert_eq!(urls.1["items"][0]["state"], "deleted");
    let repeat = call(
        &chat,
        &owner,
        "POST",
        "/chat/media/delete",
        Some(json!({"ids": [attached]})),
    )
    .await;
    assert_eq!(
        (repeat.1["deleted"].as_i64(), repeat.1["freed"].as_i64()),
        (Some(0), Some(0))
    );
    let all = call(
        &chat,
        &owner,
        "POST",
        "/chat/media/delete",
        Some(json!({"all": true})),
    )
    .await;
    assert_eq!(
        all.1,
        json!({"deleted": 3, "freed": 2 * TOTAL, "used": 0, "quota": QUOTA_BYTES})
    );
    assert_eq!(usage(&pool, &owner).await, (0, 0, 0));
    assert_eq!(
        statuses(&pool, uploaded).await[0],
        ("deleting".to_owned(), Some(true))
    );
    assert_eq!(
        statuses(&pool, reserved).await[0],
        ("deleting".to_owned(), None)
    );
    assert_eq!(
        statuses(&pool, lost).await[0],
        ("deleting".to_owned(), None)
    );
    let mine = call(&chat, &owner, "GET", "/chat/media/mine", None).await;
    assert_eq!(mine.1["items"], json!([]));
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn a_restore_within_the_window_brings_rows_and_quota_back(pool: PgPool) {
    let chat = bench(&pool);
    let owner = user();
    let group = group(&pool, &owner, &[]).await;
    let (attached, _) = seed(&pool, Seed::image(&owner, group, "attached")).await;
    add_ref(&pool, attached, group, 2).await;
    let (uploaded, uploaded_thumb) = seed(&pool, Seed::image(&owner, group, "uploaded")).await;
    let (reserved, _) = seed(&pool, Seed::image(&owner, group, "reserved")).await;
    call(
        &chat,
        &owner,
        "POST",
        "/chat/media/delete",
        Some(json!({"all": true})),
    )
    .await;
    assert_eq!(usage(&pool, &owner).await, (0, 0, 0));
    let ids = json!([uploaded_thumb.unwrap(), attached, reserved, Uuid::new_v4()]);
    let answer = call(
        &chat,
        &owner,
        "POST",
        "/chat/media/restore",
        Some(json!({"ids": ids})),
    )
    .await;
    assert_eq!(answer.0, 200, "{answer:?}");
    assert_eq!(
        answer.1,
        json!({"restored": 2, "missed": 2, "used": 2 * TOTAL, "quota": QUOTA_BYTES})
    );
    assert_eq!(usage(&pool, &owner).await, (2 * TOTAL, 0, 2));
    assert_eq!(
        statuses(&pool, attached).await,
        vec![("attached".to_owned(), None); 2]
    );
    assert_eq!(
        statuses(&pool, uploaded).await,
        vec![("uploaded".to_owned(), None); 2]
    );
    let window: bool = sqlx::query_scalar(
        "SELECT delete_after > now() + interval '59 minutes' FROM chat.media WHERE id = $1",
    )
    .bind(uploaded)
    .fetch_one(&pool)
    .await
    .expect("an attach window");
    assert!(window);
    assert_eq!(status(&pool, reserved).await.as_deref(), Some("deleting"));
    let again = call(
        &chat,
        &owner,
        "POST",
        "/chat/media/restore",
        Some(json!({"all": true})),
    )
    .await;
    assert_eq!(again.1["restored"], 0);
    assert_eq!(usage(&pool, &owner).await, (2 * TOTAL, 0, 2));
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn a_restore_over_the_quota_restores_nothing_and_a_late_one_is_missed(pool: PgPool) {
    let chat = bench(&pool);
    let owner = user();
    let group = group(&pool, &owner, &[]).await;
    let (first, _) = seed(&pool, Seed::image(&owner, group, "uploaded")).await;
    let (second, _) = seed(&pool, Seed::image(&owner, group, "uploaded")).await;
    call(
        &chat,
        &owner,
        "POST",
        "/chat/media/delete",
        Some(json!({"ids": [first, second]})),
    )
    .await;
    let full = QUOTA_BYTES as i64 - TOTAL;
    sqlx::query("UPDATE chat.usage SET bytes = $2 WHERE user_id = $1")
        .bind(&owner)
        .bind(full)
        .execute(&pool)
        .await
        .expect("a fuller quota");
    let refused = call(
        &chat,
        &owner,
        "POST",
        "/chat/media/restore",
        Some(json!({"all": true})),
    )
    .await;
    assert_eq!(error(&refused), (403, "chat_quota_exceeded"));
    assert_eq!(refused.1["used"].as_i64(), Some(full));
    assert_eq!(status(&pool, first).await.as_deref(), Some("deleting"));
    assert_eq!(status(&pool, second).await.as_deref(), Some("deleting"));
    assert_eq!(usage(&pool, &owner).await.0, full);
    let one = call(
        &chat,
        &owner,
        "POST",
        "/chat/media/restore",
        Some(json!({"ids": [first]})),
    )
    .await;
    assert_eq!(one.1["restored"], 1, "{one:?}");
    sqlx::query("UPDATE chat.media SET restorable_until = now() - interval '1 second' WHERE id = $1 OR parent_id = $1")
        .bind(second)
        .execute(&pool)
        .await
        .expect("a passed window");
    sqlx::query("UPDATE chat.usage SET bytes = 0 WHERE user_id = $1")
        .bind(&owner)
        .execute(&pool)
        .await
        .expect("room again");
    let late = call(
        &chat,
        &owner,
        "POST",
        "/chat/media/restore",
        Some(json!({"ids": [second]})),
    )
    .await;
    assert_eq!(
        (late.1["restored"].as_i64(), late.1["missed"].as_i64()),
        (Some(0), Some(1))
    );
    assert_eq!(status(&pool, second).await.as_deref(), Some("deleting"));
}
