use std::sync::Arc;
use std::time::Duration;

use sqlx::PgPool;
use uuid::Uuid;

use crate::modules::chat::ChatState;
use crate::modules::chat::conversations::fixtures::group;
use crate::modules::chat::fake_store::{FakeStore, media_on};
use crate::modules::chat::media::commit::LEFT_FOR_RECONCILE;
use crate::modules::chat::media::limits::QUOTA_BYTES;
use crate::modules::chat::media::rows::{Seed, seed, usage};
use crate::modules::chat::media::support::{
    IMAGE, THUMB, bench, call, cancel_a_lock_wait, error, redis, user, with_media,
};
use crate::modules::chat::test_support::chat_state;

pub(super) const TOTAL: i64 = (IMAGE + THUMB) as i64;

pub(super) fn on_fake(pool: &PgPool, fake: &FakeStore) -> Arc<ChatState> {
    with_media(
        chat_state(pool.clone(), redis(), true),
        media_on(&[(1, fake)]),
    )
}

pub(super) async fn keys_of(pool: &PgPool, root: Uuid) -> Vec<String> {
    sqlx::query_scalar(
        "SELECT object_key FROM chat.media WHERE id = $1 OR parent_id = $1 ORDER BY kind",
    )
    .bind(root)
    .fetch_all(pool)
    .await
    .expect("keys")
}

pub(super) fn sorted(mut keys: Vec<String>) -> Vec<String> {
    keys.sort();
    keys
}

pub(super) async fn uploaded(pool: &PgPool, fake: &FakeStore, owner: &str) -> (Uuid, Vec<String>) {
    let group = group(pool, owner, &[]).await;
    let (image, _) = seed(pool, Seed::image(owner, group, "reserved")).await;
    let uploads = keys_of(pool, image).await;
    fake.put(&uploads[0], vec![1; IMAGE as usize]);
    fake.put(&uploads[1], vec![2; THUMB as usize]);
    (image, uploads)
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn a_commit_keeps_its_own_copy_so_a_later_put_reaches_no_reader(pool: PgPool) {
    let fake = FakeStore::start(Duration::ZERO).await;
    let chat = on_fake(&pool, &fake);
    let owner = user();
    let (image, uploads) = uploaded(&pool, &fake, &owner).await;
    let uri = format!("/chat/media/{image}/commit");

    let commit = call(&chat, &owner, "POST", &uri, None).await;
    assert_eq!(commit.0, 200, "{commit:?}");
    let kept = keys_of(&pool, image).await;
    assert!(kept.iter().all(|key| !uploads.contains(key)), "{kept:?}");
    assert_eq!(fake.keys(), sorted(kept.clone()));

    fake.put(&uploads[0], vec![9; IMAGE as usize]);
    fake.put(&uploads[1], vec![8; THUMB as usize]);
    assert_eq!(call(&chat, &owner, "POST", &uri, None).await, commit);
    assert_eq!(keys_of(&pool, image).await, kept);
    assert_eq!(fake.object(&kept[0]), Some(vec![1; IMAGE as usize]));
    assert_eq!(fake.object(&kept[1]), Some(vec![2; THUMB as usize]));
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn parallel_commits_keep_one_copy_and_answer_every_caller(pool: PgPool) {
    let fake = FakeStore::start(Duration::from_millis(20)).await;
    let chat = on_fake(&pool, &fake);
    let owner = user();
    let (image, _) = uploaded(&pool, &fake, &owner).await;
    let uri = format!("/chat/media/{image}/commit");

    let commits = (0..6).map(|_| {
        let (chat, owner, uri) = (chat.clone(), owner.clone(), uri.clone());
        tokio::spawn(async move { call(&chat, &owner, "POST", &uri, None).await })
    });
    for answer in futures::future::join_all(commits).await {
        let answer = answer.expect("a commit");
        assert_eq!(answer.0, 200, "{answer:?}");
        assert_eq!(answer.1["bytes"].as_i64(), Some(TOTAL));
    }
    assert_eq!(usage(&pool, &owner).await, (TOTAL, 0, 1));
    let kept = keys_of(&pool, image).await;
    assert_eq!(fake.keys(), sorted(kept.clone()));
    assert_eq!(fake.object(&kept[0]), Some(vec![1; IMAGE as usize]));
}

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

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn a_failed_copy_keeps_the_upload_key_and_a_repeat_commit_finishes(pool: PgPool) {
    let fake = FakeStore::start(Duration::ZERO).await;
    let chat = on_fake(&pool, &fake);
    let owner = user();
    let (image, uploads) = uploaded(&pool, &fake, &owner).await;
    let uri = format!("/chat/media/{image}/commit");

    fake.fail_copies(true);
    let failed = call(&chat, &owner, "POST", &uri, None).await;
    assert_eq!(error(&failed), (503, "chat_busy"), "{failed:?}");
    assert_eq!(keys_of(&pool, image).await, uploads);
    assert_eq!(usage(&pool, &owner).await, (0, TOTAL, 0));
    assert_eq!(fake.keys(), sorted(uploads.clone()));

    fake.fail_copies(false);
    let done = call(&chat, &owner, "POST", &uri, None).await;
    assert_eq!(done.0, 200, "{done:?}");
    let kept = keys_of(&pool, image).await;
    assert!(kept.iter().all(|key| !uploads.contains(key)), "{kept:?}");
    assert_eq!(fake.keys(), sorted(kept));
    assert_eq!(usage(&pool, &owner).await, (TOTAL, 0, 1));
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn a_flip_that_fails_in_postgres_drops_its_copies(pool: PgPool) {
    let fake = FakeStore::start(Duration::ZERO).await;
    let chat = on_fake(&pool, &fake);
    let owner = user();
    let (image, uploads) = uploaded(&pool, &fake, &owner).await;
    let mut blocker = pool.begin().await.expect("a tx");
    sqlx::query("SELECT id FROM chat.media WHERE id = $1 FOR UPDATE")
        .bind(image)
        .execute(&mut *blocker)
        .await
        .expect("a held row");
    let committing = {
        let (chat, owner) = (chat.clone(), owner.clone());
        tokio::spawn(async move {
            call(
                &chat,
                &owner,
                "POST",
                &format!("/chat/media/{image}/commit"),
                None,
            )
            .await
        })
    };
    cancel_a_lock_wait(&pool, None)
        .await
        .expect("a cancelled flip");
    let failed = committing.await.expect("a commit");
    blocker.rollback().await.expect("a rollback");

    assert!(failed.0 >= 500, "{failed:?}");
    assert_eq!(keys_of(&pool, image).await, uploads);
    assert_eq!(fake.keys(), sorted(uploads));
    assert_eq!(usage(&pool, &owner).await, (0, TOTAL, 0));
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn an_upload_key_the_store_will_not_delete_is_counted_and_logged(pool: PgPool) {
    crate::metrics::init();
    let fake = FakeStore::start(Duration::ZERO).await;
    let chat = on_fake(&pool, &fake);
    let owner = user();
    let (image, uploads) = uploaded(&pool, &fake, &owner).await;
    let before = left_for_reconcile();
    fake.fail_deletes(true);

    let commit = call(
        &chat,
        &owner,
        "POST",
        &format!("/chat/media/{image}/commit"),
        None,
    )
    .await;
    assert_eq!(commit.0, 200, "{commit:?}");
    assert!(left_for_reconcile() - before >= 2.0);
    let mut kept = keys_of(&pool, image).await;
    kept.extend(uploads);
    assert_eq!(fake.keys(), sorted(kept));
}

pub(super) fn left_for_reconcile() -> f64 {
    crate::metrics::render_for_tests()
        .unwrap_or_default()
        .lines()
        .filter(|line| line.starts_with(LEFT_FOR_RECONCILE))
        .filter_map(|line| line.rsplit(' ').next()?.parse::<f64>().ok())
        .sum()
}
