use std::time::{Duration, Instant};

use sqlx::PgPool;
use uuid::Uuid;

use crate::modules::chat::ChatResult;
use crate::modules::chat::conversations::fixtures::group;
use crate::modules::chat::media::ledger;
use crate::modules::chat::media::rows::{Seed, seed, status, usage};
use crate::modules::chat::media::support::{IMAGE, THUMB, bench, until_blocked, user};
use crate::modules::chat::media::wire::Picked;
use crate::modules::chat::stores::{self, Object};
use crate::modules::chat::sweeps::budget::Budget;
use crate::modules::chat::sweeps::media::expire;

const TOTAL: i64 = (IMAGE + THUMB) as i64;

fn kept_pair(root: Uuid, thumb: Uuid) -> Vec<Object> {
    vec![
        Object {
            id: root,
            key: stores::new_key(),
            bytes: IMAGE,
        },
        Object {
            id: thumb,
            key: stores::new_key(),
            bytes: THUMB,
        },
    ]
}

async fn key_of(pool: &PgPool, id: Uuid) -> String {
    sqlx::query_scalar("SELECT object_key FROM chat.media WHERE id = $1")
        .bind(id)
        .fetch_one(pool)
        .await
        .expect("a key")
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn parallel_commits_move_the_reservation_once(pool: PgPool) {
    let owner = user();
    let group = group(&pool, &owner, &[]).await;
    let (root, thumb) = seed(&pool, Seed::image(&owner, group, "reserved")).await;
    let thumb = thumb.expect("a thumb");
    let tasks = (0..12).map(|_| {
        let (pool, owner) = (pool.clone(), owner.clone());
        let kept = kept_pair(root, thumb);
        tokio::spawn(async move {
            let moved = ledger::commit(&pool, &owner, root, &kept)
                .await
                .expect("a commit");
            moved.map(|_| kept)
        })
    });
    let won: Vec<Vec<Object>> = futures::future::join_all(tasks)
        .await
        .into_iter()
        .filter_map(|done| done.expect("a task"))
        .collect();
    assert_eq!(won.len(), 1);
    assert_eq!(usage(&pool, &owner).await, (TOTAL, 0, 1));
    assert_eq!(status(&pool, thumb).await.as_deref(), Some("uploaded"));
    for object in &won[0] {
        assert_eq!(key_of(&pool, object.id).await, object.key);
    }
    let (expired, _) = seed(&pool, Seed::image(&owner, group, "reserved")).await;
    sqlx::query("UPDATE chat.media SET delete_after = now() - interval '1 second' WHERE id = $1 OR parent_id = $1")
        .bind(expired)
        .execute(&pool)
        .await
        .expect("an expired reservation");
    assert_eq!(
        ledger::commit(&pool, &owner, expired, &kept_pair(expired, Uuid::new_v4()))
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
            ledger::release(&pool, &owner, &picked)
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
    ledger::release(&pool, &owner, &picked)
        .await
        .expect("a delete");
    sqlx::query("UPDATE chat.media SET lease_until = now() + interval '5 minutes' WHERE id = $1 OR parent_id = $1")
        .bind(root)
        .execute(&pool)
        .await
        .expect("a claim");
    let restored = ledger::restore(&pool, &owner, &picked)
        .await
        .expect("a restore");
    assert_eq!(restored.roots, 0);
    assert_eq!(status(&pool, root).await.as_deref(), Some("deleting"));
    assert_eq!(usage(&pool, &owner).await, (0, 0, 0));
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn a_restore_with_nothing_left_to_restore_misses_instead_of_refusing(pool: PgPool) {
    let owner = user();
    let group = group(&pool, &owner, &[]).await;
    let (root, thumb) = seed(&pool, Seed::image(&owner, group, "uploaded")).await;
    let picked = Picked::Ids(vec![root]);
    ledger::release(&pool, &owner, &picked)
        .await
        .expect("a delete");
    sqlx::query("UPDATE chat.media SET lease_until = now() + interval '5 minutes' WHERE id = $1")
        .bind(thumb.expect("a thumb"))
        .execute(&pool)
        .await
        .expect("a claimed thumb");
    let restored = ledger::restore(&pool, &owner, &picked)
        .await
        .expect("a miss, not a refusal");
    assert_eq!((restored.roots, restored.used.total()), (0, 0));
    assert_eq!(status(&pool, root).await.as_deref(), Some("deleting"));
    assert_eq!(usage(&pool, &owner).await, (0, 0, 0));
}

async fn locked_waiters(pool: &PgPool, sessions: i64) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let waiting: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM pg_stat_activity
             WHERE datname = current_database() AND wait_event_type = 'Lock'",
        )
        .fetch_one(pool)
        .await
        .expect("pg_stat_activity");
        if waiting >= sessions {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "{waiting} of {sessions} sessions wait"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

async fn hold_usage(pool: &PgPool, owner: &str) -> sqlx::Transaction<'static, sqlx::Postgres> {
    let mut held = pool.begin().await.expect("a tx");
    sqlx::query("SELECT 1 FROM chat.usage WHERE user_id = $1 FOR UPDATE")
        .bind(owner)
        .execute(&mut *held)
        .await
        .expect("a held usage row");
    held
}

async fn release(pool: PgPool, roots: Vec<Uuid>) -> ChatResult<u64> {
    let mut tx = pool.begin().await?;
    let settled = ledger::settle(&mut tx, &roots).await?;
    tx.commit().await?;
    Ok(settled)
}

fn two_owners() -> (String, String) {
    let mut owners = [user(), user()];
    owners.sort();
    let [low, high] = owners;
    (low, high)
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn releases_over_two_owners_in_opposite_order_never_deadlock(pool: PgPool) {
    let (low, high) = two_owners();
    let group = group(&pool, &low, &[]).await;
    for held in [&low, &high, &low, &high] {
        let mut roots = Vec::new();
        for owner in [&low, &high, &high, &low] {
            roots.push(seed(&pool, Seed::image(owner, group, "attached")).await.0);
        }
        let blocker = hold_usage(&pool, held).await;
        let first = tokio::spawn(release(pool.clone(), roots[..2].to_vec()));
        locked_waiters(&pool, 1).await;
        let second = tokio::spawn(release(pool.clone(), roots[2..].to_vec()));
        locked_waiters(&pool, 2).await;
        blocker.commit().await.expect("a commit");
        for release in [first, second] {
            assert_eq!(release.await.expect("a task"), Ok(2));
        }
        for root in roots {
            assert_eq!(status(&pool, root).await.as_deref(), Some("deleting"));
        }
    }
    assert_eq!(usage(&pool, &low).await, (0, 0, 0));
    assert_eq!(usage(&pool, &high).await, (0, 0, 0));
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn an_expiry_and_a_release_over_the_same_owners_never_deadlock(pool: PgPool) {
    let (low, high) = two_owners();
    let group = group(&pool, &low, &[]).await;
    let chat = bench(&pool);
    for held in [&high, &low] {
        let mut attached = Vec::new();
        for owner in [&low, &high] {
            attached.push(seed(&pool, Seed::image(owner, group, "attached")).await.0);
        }
        for owner in [&high, &low] {
            let (stale, _) = seed(&pool, Seed::image(owner, group, "reserved")).await;
            sqlx::query("UPDATE chat.media SET delete_after = now() - interval '1 second' WHERE id = $1 OR parent_id = $1")
                .bind(stale)
                .execute(&pool)
                .await
                .expect("a stale reservation");
        }
        let blocker = hold_usage(&pool, held).await;
        let releasing = tokio::spawn(release(pool.clone(), attached));
        locked_waiters(&pool, 1).await;
        let expiring = {
            let chat = chat.clone();
            tokio::spawn(async move { expire::run(&chat, &mut Budget::new(1)).await })
        };
        locked_waiters(&pool, 2).await;
        blocker.commit().await.expect("a commit");
        assert_eq!(releasing.await.expect("a task"), Ok(2));
        assert_eq!(expiring.await.expect("a task"), Ok(2));
    }
    assert_eq!(usage(&pool, &low).await, (0, 0, 0));
    assert_eq!(usage(&pool, &high).await, (0, 0, 0));
}
