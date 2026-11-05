use std::time::{Duration, Instant};

use sqlx::PgPool;
use uuid::Uuid;

use super::support::{conversation, state, user};
use crate::modules::chat::media::ledger;
use crate::modules::chat::media::support::until_blocked;
use crate::modules::chat::media::wire::Picked;
use crate::modules::chat::sweeps::budget::Budget;
use crate::modules::chat::sweeps::media::fixtures::{
    attach, ok, owner_delete, refs, statuses, upload, usage,
};
use crate::modules::chat::sweeps::media::withdrawn;

async fn window_passed(pg: &PgPool, root: Uuid) -> anyhow::Result<()> {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let passed: bool = sqlx::query_scalar(
            "SELECT restorable_until <= clock_timestamp() FROM chat.media WHERE id = $1",
        )
        .bind(root)
        .fetch_one(pg)
        .await?;
        if passed {
            return Ok(());
        }
        anyhow::ensure!(Instant::now() < deadline, "the restore window never passed");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
#[ignore = "requires a local Redis"]
async fn a_restore_begun_in_the_window_keeps_its_image_when_the_sweep_runs_after(
    pg: PgPool,
) -> anyhow::Result<()> {
    let chat = state(pg.clone());
    let owner = user();
    let group = conversation(&pg, "group", &owner).await?;
    let image = upload(&pg, &owner, group, 1).await?;
    assert!(attach(&pg, image.id, group, 1).await?);
    let charged = usage(&pg, &owner).await?;
    assert!(owner_delete(&pg, image.id, 2).await?);
    let mut blocker = pg.begin().await?;
    sqlx::query("SELECT 1 FROM chat.usage WHERE user_id = $1 FOR UPDATE")
        .bind(&owner)
        .execute(&mut *blocker)
        .await?;
    let restoring = {
        let (pg, owner, picked) = (pg.clone(), owner.clone(), Picked::Ids(vec![image.id]));
        tokio::spawn(async move { ledger::restore(&pg, &owner, &picked).await })
    };
    until_blocked(&pg).await;
    window_passed(&pg, image.id).await?;

    let swept = tokio::time::timeout(
        Duration::from_secs(5),
        withdrawn::run(&chat, &mut Budget::new(5)),
    )
    .await?;
    assert_eq!(ok(swept)?, 0, "the sweep skips a row a restore holds");
    blocker.commit().await?;
    let restored = ok(restoring.await?)?;

    assert_eq!(restored.roots, 1);
    assert_eq!(
        restored.used.total(),
        u64::try_from(charged.bytes + charged.reserved)?
    );
    assert_eq!(usage(&pg, &owner).await?, charged);
    assert_eq!(
        statuses(&pg, &image).await?,
        vec![Some("attached".to_owned()); 2]
    );
    assert_eq!(refs(&pg, image.id).await?, 1);
    assert_eq!(ok(withdrawn::run(&chat, &mut Budget::new(5)).await)?, 0);
    assert_eq!(refs(&pg, image.id).await?, 1);
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
#[ignore = "requires a local Redis"]
async fn a_restore_keeps_its_start_as_the_cutoff_and_misses_what_the_sweep_took(
    pg: PgPool,
) -> anyhow::Result<()> {
    let chat = state(pg.clone());
    let owner = user();
    let group = conversation(&pg, "group", &owner).await?;
    let mut images = vec![
        upload(&pg, &owner, group, 1).await?,
        upload(&pg, &owner, group, 1).await?,
    ];
    images.sort_by_key(|image| image.id);
    for (seq, image) in images.iter().enumerate() {
        assert!(attach(&pg, image.id, group, seq as i64 + 1).await?);
    }
    let one = usage(&pg, &owner).await?.bytes / 2;
    for image in &images {
        assert!(owner_delete(&pg, image.id, 2).await?);
    }
    let (first, second) = (&images[0], &images[1]);
    let mut blocker = pg.begin().await?;
    sqlx::query("SELECT 1 FROM chat.media WHERE id = $1 FOR UPDATE")
        .bind(first.id)
        .execute(&mut *blocker)
        .await?;
    let restoring = {
        let (pg, owner) = (pg.clone(), owner.clone());
        let picked = Picked::Ids(vec![second.id, first.id]);
        tokio::spawn(async move { ledger::restore(&pg, &owner, &picked).await })
    };
    until_blocked(&pg).await;
    window_passed(&pg, second.id).await?;

    let swept = tokio::time::timeout(
        Duration::from_secs(5),
        withdrawn::run(&chat, &mut Budget::new(5)),
    )
    .await?;
    assert_eq!(ok(swept)?, 1, "only the row nobody holds");
    blocker.commit().await?;
    let restored = ok(restoring.await?)?;

    assert_eq!(restored.roots, 1);
    assert_eq!(
        statuses(&pg, first).await?,
        vec![Some("attached".to_owned()); 2]
    );
    assert_eq!(refs(&pg, first.id).await?, 1);
    assert_eq!(
        statuses(&pg, second).await?,
        vec![Some("deleting".to_owned()); 2]
    );
    assert_eq!(refs(&pg, second.id).await?, 0);
    assert_eq!(usage(&pg, &owner).await?.bytes, one);
    Ok(())
}
