use sqlx::PgPool;
use uuid::Uuid;

use crate::modules::chat::media::hooks;
use crate::modules::chat::media::rows::{Seed, add_ref, seed, status, usage};
use crate::modules::chat::media::support::{until_blocked, user};

async fn set(pool: &PgPool, root: Uuid, sql: &str) {
    sqlx::query(&format!(
        "UPDATE chat.media SET {sql} WHERE id = $1 OR parent_id = $1"
    ))
    .bind(root)
    .execute(pool)
    .await
    .expect("a row change");
}

async fn holds_of(pool: &PgPool, report: Uuid) -> Vec<Uuid> {
    sqlx::query_scalar("SELECT media_id FROM chat.media_holds WHERE report_id = $1 ORDER BY 1")
        .bind(report)
        .fetch_all(pool)
        .await
        .expect("holds")
}

async fn hold(pool: &PgPool, report: Uuid, roots: &[Uuid]) -> Vec<Uuid> {
    let mut tx = pool.begin().await.expect("a tx");
    let held = hooks::hold(&mut tx, report, roots).await.expect("a hold");
    tx.commit().await.expect("a commit");
    held
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn a_hold_takes_live_rows_only_and_keeps_an_unclaimed_deleting_row_held(pool: PgPool) {
    let owner = user();
    let origin = Uuid::now_v7();
    let mut roots = Vec::new();
    let mut thumbs = Vec::new();
    for shape in [
        "attached", "uploaded", "held", "free", "claimed", "window", "lost", "reserved",
    ] {
        let stored = match shape {
            "free" | "claimed" | "window" => "deleting",
            other => other,
        };
        let (root, thumb) = seed(&pool, Seed::image(&owner, origin, stored)).await;
        match shape {
            "claimed" => set(&pool, root, "lease_until = now() + interval '5 minutes'").await,
            "window" => {
                set(
                    &pool,
                    root,
                    "restorable_until = now() + interval '15 seconds'",
                )
                .await
            }
            _ => {}
        }
        roots.push(root);
        thumbs.push(thumb.expect("a thumb"));
    }
    let report = Uuid::now_v7();
    let mut live = roots[..4].to_vec();
    live.push(roots[5]);
    live.sort();

    assert_eq!(hold(&pool, report, &roots).await, live);
    assert_eq!(holds_of(&pool, report).await, live);
    let expected = [
        "attached", "uploaded", "held", "held", "deleting", "deleting", "lost", "reserved",
    ];
    for ((root, thumb), wanted) in roots.iter().zip(&thumbs).zip(expected) {
        assert_eq!(status(&pool, *root).await.as_deref(), Some(wanted));
        assert_eq!(status(&pool, *thumb).await.as_deref(), Some(wanted));
    }
    assert_eq!(
        hold(&pool, report, &roots).await,
        live,
        "a second hold is the same"
    );
    assert_eq!(holds_of(&pool, report).await, live);
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn a_settle_waits_for_a_hold_in_flight_and_keeps_the_row_held(pool: PgPool) {
    let owner = user();
    let conversation = Uuid::now_v7();
    let (root, thumb) = seed(&pool, Seed::image(&owner, conversation, "attached")).await;
    add_ref(&pool, root, conversation, 5).await;
    let report = Uuid::now_v7();
    let mut holding = pool.begin().await.expect("a tx");
    assert_eq!(
        hooks::hold(&mut holding, report, &[root])
            .await
            .expect("a hold"),
        vec![root]
    );
    let dropping = {
        let pool = pool.clone();
        tokio::spawn(async move {
            let mut tx = pool.begin().await.expect("a tx");
            let settled = hooks::drop_for_messages(&mut tx, conversation, &[5])
                .await
                .expect("a drop");
            tx.commit().await.expect("a commit");
            settled
        })
    };
    until_blocked(&pool).await;
    holding.commit().await.expect("the hold lands");
    assert_eq!(dropping.await.expect("the drop"), 1);
    assert_eq!(status(&pool, root).await.as_deref(), Some("held"));
    assert_eq!(
        status(&pool, thumb.expect("a thumb")).await.as_deref(),
        Some("held")
    );
    assert_eq!(usage(&pool, &owner).await, (0, 0, 0), "released once");
}
