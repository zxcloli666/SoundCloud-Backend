use sqlx::PgPool;

use crate::modules::chat::conversations::fixtures::group;
use crate::modules::chat::media::ledger;
use crate::modules::chat::media::rows::{Seed, seed, status, usage};
use crate::modules::chat::media::support::{IMAGE, THUMB, until_blocked, user};
use crate::modules::chat::media::wire::Picked;
use crate::modules::chat::quota;

const TOTAL: i64 = (IMAGE + THUMB) as i64;

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn parallel_commits_move_the_reservation_once(pool: PgPool) {
    let owner = user();
    let group = group(&pool, &owner, &[]).await;
    let (root, thumb) = seed(&pool, Seed::image(&owner, group, "reserved")).await;
    let tasks = (0..12).map(|_| {
        let (pool, owner) = (pool.clone(), owner.clone());
        tokio::spawn(async move { quota::commit(&pool, &owner, root).await.expect("a commit") })
    });
    let moved = futures::future::join_all(tasks)
        .await
        .into_iter()
        .filter(|done| done.as_ref().expect("a task").is_some())
        .count();
    assert_eq!(moved, 1);
    assert_eq!(usage(&pool, &owner).await, (TOTAL, 0, 1));
    assert_eq!(
        status(&pool, thumb.unwrap()).await.as_deref(),
        Some("uploaded")
    );
    let (expired, _) = seed(&pool, Seed::image(&owner, group, "reserved")).await;
    sqlx::query("UPDATE chat.media SET delete_after = now() - interval '1 second' WHERE id = $1 OR parent_id = $1")
        .bind(expired)
        .execute(&pool)
        .await
        .expect("an expired reservation");
    assert_eq!(
        quota::commit(&pool, &owner, expired)
            .await
            .expect("a commit"),
        None
    );
    assert_eq!(usage(&pool, &owner).await, (TOTAL, TOTAL, 1));
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn a_delete_waiting_on_a_status_flip_still_deletes_the_new_status(pool: PgPool) {
    let owner = user();
    let group = group(&pool, &owner, &[]).await;
    let (root, _) = seed(&pool, Seed::image(&owner, group, "uploaded")).await;
    let mut flip = pool.begin().await.expect("a tx");
    ledger::lock(&mut flip, &[root]).await.expect("a lock");
    sqlx::query("UPDATE chat.media SET status = 'attached', delete_after = NULL WHERE id = $1 OR parent_id = $1")
        .bind(root)
        .execute(&mut *flip)
        .await
        .expect("an attach");
    let picked = Picked::Ids(vec![root]);
    let deleting = {
        let (pool, owner) = (pool.clone(), owner.clone());
        tokio::spawn(async move {
            quota::release(&pool, &owner, &picked)
                .await
                .expect("a delete")
        })
    };
    until_blocked(&pool).await;
    assert!(!deleting.is_finished());
    flip.commit().await.expect("a commit");
    let (released, used) = deleting.await.expect("a task");
    assert_eq!(
        (released.roots, released.freed, used.total()),
        (1, TOTAL as u64, 0)
    );
    assert_eq!(status(&pool, root).await.as_deref(), Some("deleting"));
    assert_eq!(usage(&pool, &owner).await, (0, 0, 0));
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn a_restore_never_takes_back_a_row_the_sweeper_claimed(pool: PgPool) {
    let owner = user();
    let group = group(&pool, &owner, &[]).await;
    let (root, _) = seed(&pool, Seed::image(&owner, group, "uploaded")).await;
    let picked = Picked::Ids(vec![root]);
    quota::release(&pool, &owner, &picked)
        .await
        .expect("a delete");
    sqlx::query("UPDATE chat.media SET lease_until = now() + interval '5 minutes' WHERE id = $1 OR parent_id = $1")
        .bind(root)
        .execute(&pool)
        .await
        .expect("a claim");
    let restored = quota::restore(&pool, &owner, &picked)
        .await
        .expect("a restore");
    assert_eq!(restored.roots, 0);
    assert_eq!(status(&pool, root).await.as_deref(), Some("deleting"));
    assert_eq!(usage(&pool, &owner).await, (0, 0, 0));
}
