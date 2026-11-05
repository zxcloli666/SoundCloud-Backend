use std::time::Duration;

use chrono::{DateTime, Utc};
use sqlx::PgPool;

use super::tests::{TOTAL, keys_of, left_for_reconcile, on_fake, sorted, uploaded};
use crate::modules::chat::fake_store::FakeStore;
use crate::modules::chat::media::pg_cut::{Cut, PgCut};
use crate::modules::chat::media::rows::usage;
use crate::modules::chat::media::support::{IMAGE, THUMB, call, cancel_a_lock_wait, error, user};

async fn until_waiting(pool: &PgPool, count: i64) {
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        let waiting: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM pg_stat_activity
             WHERE datname = current_database() AND wait_event_type = 'Lock'",
        )
        .fetch_one(pool)
        .await
        .expect("pg_stat_activity");
        if waiting >= count {
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "{waiting} of {count} sessions wait on a lock"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn a_usage_read_that_fails_after_the_flip_keeps_the_copies_and_asks_for_a_retry(
    pool: PgPool,
) {
    let fake = FakeStore::start(Duration::ZERO).await;
    let chat = on_fake(&pool, &fake);
    let owner = user();
    let (image, uploads) = uploaded(&pool, &fake, &owner).await;
    sqlx::query("DELETE FROM chat.usage WHERE user_id = $1")
        .bind(&owner)
        .execute(&pool)
        .await
        .expect("no usage row");
    let mut share = pool.begin().await.expect("a tx");
    sqlx::query("LOCK TABLE chat.usage IN SHARE MODE")
        .execute(&mut *share)
        .await
        .expect("a share lock");
    let uri = format!("/chat/media/{image}/commit");
    let committing = {
        let (chat, owner, uri) = (chat.clone(), owner.clone(), uri.clone());
        tokio::spawn(async move { call(&chat, &owner, "POST", &uri, None).await })
    };
    until_waiting(&pool, 1).await;
    let mut exclusive = pool.begin().await.expect("a tx");
    let locking = tokio::spawn(async move {
        sqlx::query("LOCK TABLE chat.usage IN ACCESS EXCLUSIVE MODE")
            .execute(&mut *exclusive)
            .await
            .expect("an exclusive lock");
        exclusive
    });
    until_waiting(&pool, 2).await;
    let queued: DateTime<Utc> = sqlx::query_scalar("SELECT clock_timestamp()")
        .fetch_one(&pool)
        .await
        .expect("a clock");
    share.rollback().await.expect("a rollback");
    cancel_a_lock_wait(&pool, Some(queued))
        .await
        .expect("a cancelled usage read");
    let failed = committing.await.expect("a commit");
    locking
        .await
        .expect("an exclusive tx")
        .rollback()
        .await
        .expect("a rollback");

    assert_eq!(error(&failed), (503, "chat_busy"), "{failed:?}");
    let kept = keys_of(&pool, image).await;
    assert!(kept.iter().all(|key| !uploads.contains(key)), "{kept:?}");
    assert_eq!(fake.keys(), sorted(kept.clone()));

    let retried = call(&chat, &owner, "POST", &uri, None).await;
    assert_eq!(retried.0, 200, "{retried:?}");
    assert_eq!(retried.1["bytes"].as_i64(), Some(TOTAL));
    assert_eq!(keys_of(&pool, image).await, kept);
    assert_eq!(fake.keys(), sorted(kept));
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn a_commit_whose_reply_is_lost_keeps_the_copies_the_row_points_at(pool: PgPool) {
    let fake = FakeStore::start(Duration::ZERO).await;
    let cut = PgCut::start(&pool).await;
    let chat = on_fake(&cut.pool, &fake);
    let owner = user();
    let (image, uploads) = uploaded(&pool, &fake, &owner).await;
    let uri = format!("/chat/media/{image}/commit");

    cut.arm(Cut::AfterCommit);
    let first = call(&chat, &owner, "POST", &uri, None).await;
    assert!(cut.fired());
    assert_eq!(first.0, 200, "{first:?}");
    let kept = keys_of(&pool, image).await;
    assert!(kept.iter().all(|key| !uploads.contains(key)), "{kept:?}");
    assert_eq!(fake.keys(), sorted(kept.clone()));
    assert_eq!(usage(&pool, &owner).await, (TOTAL, 0, 1));

    assert_eq!(call(&chat, &owner, "POST", &uri, None).await, first);
    assert_eq!(keys_of(&pool, image).await, kept);
    assert_eq!(fake.keys(), sorted(kept));
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn a_connection_lost_before_commit_keeps_every_copy_and_a_retry_finishes(pool: PgPool) {
    crate::metrics::init();
    let fake = FakeStore::start(Duration::ZERO).await;
    let cut = PgCut::start(&pool).await;
    let chat = on_fake(&cut.pool, &fake);
    let owner = user();
    let (image, uploads) = uploaded(&pool, &fake, &owner).await;
    let uri = format!("/chat/media/{image}/commit");
    let before = left_for_reconcile();

    cut.arm(Cut::BeforeCommit);
    let failed = call(&chat, &owner, "POST", &uri, None).await;
    assert!(cut.fired());
    assert_eq!(error(&failed), (503, "chat_busy"), "{failed:?}");
    assert_eq!(keys_of(&pool, image).await, uploads);
    let stale = copies_beside(&fake, &uploads);
    assert_eq!(stale.len(), 2, "{:?}", fake.keys());
    assert_eq!(
        fake.keys(),
        sorted([uploads.clone(), stale.clone()].concat())
    );
    assert!(left_for_reconcile() - before >= 4.0);
    assert_eq!(usage(&pool, &owner).await, (0, TOTAL, 0));

    let retried = call(&chat, &owner, "POST", &uri, None).await;
    assert_eq!(retried.0, 200, "{retried:?}");
    let kept = keys_of(&pool, image).await;
    assert!(
        kept.iter()
            .all(|key| !uploads.contains(key) && !stale.contains(key)),
        "{kept:?}"
    );
    assert_eq!(fake.keys(), sorted([kept, stale].concat()));
    assert_eq!(usage(&pool, &owner).await, (TOTAL, 0, 1));
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn a_commit_still_in_flight_at_the_re_read_keeps_the_copies_it_lands_on(pool: PgPool) {
    crate::metrics::init();
    let fake = FakeStore::start(Duration::ZERO).await;
    let cut = PgCut::start(&pool).await;
    let chat = on_fake(&cut.pool, &fake);
    let owner = user();
    let (image, uploads) = uploaded(&pool, &fake, &owner).await;
    let uri = format!("/chat/media/{image}/commit");
    let before = left_for_reconcile();

    cut.arm(Cut::HoldCommit);
    let failed = call(&chat, &owner, "POST", &uri, None).await;
    assert!(cut.fired());
    assert_eq!(error(&failed), (503, "chat_busy"), "{failed:?}");
    assert_eq!(keys_of(&pool, image).await, uploads);
    let copies = copies_beside(&fake, &uploads);
    assert_eq!(copies.len(), 2, "{:?}", fake.keys());
    assert_eq!(
        fake.keys(),
        sorted([uploads.clone(), copies.clone()].concat())
    );
    assert!(left_for_reconcile() - before >= 4.0);

    cut.release();
    cut.landed().await;
    let kept = keys_of(&pool, image).await;
    assert_eq!(sorted(kept.clone()), sorted(copies));
    assert_eq!(fake.object(&kept[0]), Some(vec![1; IMAGE as usize]));
    assert_eq!(fake.object(&kept[1]), Some(vec![2; THUMB as usize]));
    assert_eq!(fake.keys(), sorted([kept, uploads].concat()));
    assert_eq!(usage(&pool, &owner).await, (TOTAL, 0, 1));

    let retried = call(&chat, &owner, "POST", &uri, None).await;
    assert_eq!(retried.0, 200, "{retried:?}");
    assert_eq!(retried.1["bytes"].as_i64(), Some(TOTAL));
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn a_commit_whose_row_is_gone_by_the_re_read_drops_every_copy(pool: PgPool) {
    let fake = FakeStore::start(Duration::ZERO).await;
    let cut = PgCut::start(&pool).await;
    let chat = on_fake(&cut.pool, &fake);
    let owner = user();
    let (image, _) = uploaded(&pool, &fake, &owner).await;
    let uri = format!("/chat/media/{image}/commit");

    cut.arm(Cut::HoldReply);
    let committing = {
        let (chat, owner, uri) = (chat.clone(), owner.clone(), uri.clone());
        tokio::spawn(async move { call(&chat, &owner, "POST", &uri, None).await })
    };
    cut.landed().await;
    sqlx::query("DELETE FROM chat.media WHERE id = $1 OR parent_id = $1")
        .bind(image)
        .execute(&pool)
        .await
        .expect("a dropped row");
    cut.release();
    let gone = committing.await.expect("a commit");

    assert!(cut.fired());
    assert_eq!(error(&gone), (404, "chat_not_found"), "{gone:?}");
    assert_eq!(fake.keys(), Vec::<String>::new());
}

fn copies_beside(fake: &FakeStore, uploads: &[String]) -> Vec<String> {
    fake.keys()
        .into_iter()
        .filter(|key| !uploads.contains(key))
        .collect()
}
