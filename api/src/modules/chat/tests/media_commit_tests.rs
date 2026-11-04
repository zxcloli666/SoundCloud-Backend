use sqlx::PgPool;
use uuid::Uuid;

use crate::modules::chat::conversations::fixtures::group;
use crate::modules::chat::media::limits::QUOTA_BYTES;
use crate::modules::chat::media::rows::{Seed, seed};
use crate::modules::chat::media::support::{IMAGE, THUMB, bench, call, error, user};

const TOTAL: i64 = (IMAGE + THUMB) as i64;

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn a_commit_answers_only_the_owner_and_repeats_after_success(pool: PgPool) {
    let chat = bench(&pool);
    let (owner, other) = (user(), user());
    let group = group(&pool, &owner, &[]).await;
    let (uploaded, thumb) = seed(&pool, Seed::image(&owner, group, "uploaded")).await;
    let (attached, _) = seed(&pool, Seed::image(&owner, group, "attached")).await;
    for id in [uploaded, attached] {
        let answer = call(
            &chat,
            &owner,
            "POST",
            &format!("/chat/media/{id}/commit"),
            None,
        )
        .await;
        assert_eq!(answer.0, 200, "{answer:?}");
        assert_eq!(answer.1["id"], serde_json::json!(id));
        assert_eq!(answer.1["bytes"].as_i64(), Some(TOTAL));
        assert_eq!(
            (answer.1["used"].as_i64(), answer.1["quota"].as_u64()),
            (Some(2 * TOTAL), Some(QUOTA_BYTES))
        );
    }
    for (who, id) in [
        (&other, uploaded),
        (&owner, Uuid::new_v4()),
        (&owner, thumb.unwrap()),
    ] {
        let answer = call(
            &chat,
            who,
            "POST",
            &format!("/chat/media/{id}/commit"),
            None,
        )
        .await;
        assert_eq!(error(&answer), (404, "chat_not_found"));
    }
    for state in ["deleting", "held", "lost"] {
        let (id, _) = seed(&pool, Seed::image(&owner, group, state)).await;
        let answer = call(
            &chat,
            &owner,
            "POST",
            &format!("/chat/media/{id}/commit"),
            None,
        )
        .await;
        assert_eq!(error(&answer), (404, "chat_not_found"), "{state}");
    }
    let (expired, _) = seed(&pool, Seed::image(&owner, group, "reserved")).await;
    sqlx::query("UPDATE chat.media SET delete_after = now() - interval '1 second' WHERE id = $1 OR parent_id = $1")
        .bind(expired)
        .execute(&pool)
        .await
        .expect("an expired reservation");
    let answer = call(
        &chat,
        &owner,
        "POST",
        &format!("/chat/media/{expired}/commit"),
        None,
    )
    .await;
    assert_eq!(error(&answer), (404, "chat_not_found"));
}
