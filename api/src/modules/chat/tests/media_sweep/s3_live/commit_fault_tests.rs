use sqlx::PgPool;
use uuid::Uuid;

use super::reconcile_tests::group;
use super::{bench, bucket, days_later, listing, put_objects, store};
use crate::modules::chat::media::pg_cut::{Cut, PgCut, within};
use crate::modules::chat::media::support::{call, error};
use crate::modules::chat::sweeps::budget::Budget;
use crate::modules::chat::sweeps::media::fixtures::{Upload, Usage, ok, reserve, statuses, usage};
use crate::modules::chat::sweeps::media::reconcile;

async fn keys_of(pg: &PgPool, root: Uuid) -> anyhow::Result<Vec<String>> {
    let mut keys: Vec<String> =
        sqlx::query_scalar("SELECT object_key FROM chat.media WHERE id = $1 OR parent_id = $1")
            .bind(root)
            .fetch_all(pg)
            .await?;
    keys.sort();
    Ok(keys)
}

fn uploads_of(pending: &Upload) -> Vec<String> {
    let mut keys: Vec<String> = pending.objects().into_iter().map(|(key, _)| key).collect();
    keys.sort();
    keys
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
#[ignore = "needs CHAT_TEST_S3_ENDPOINT (SeaweedFS)"]
async fn copies_of_a_commit_that_never_landed_go_and_the_upload_stays(
    pg: PgPool,
) -> anyhow::Result<()> {
    let name = bucket(0).await?;
    let cut = PgCut::start(&pg).await;
    let b = bench(cut.pool.clone(), &[store(1, &name)], 1).await?;
    let owner = "uncommitted";
    let chat_id = group(&pg).await?;
    let pending = reserve(&pg, owner, chat_id, 1, true).await?;
    put_objects(&b.chat, &pending).await?;
    let uploads = uploads_of(&pending);
    let uri = format!("/chat/media/{}/commit", pending.id);

    cut.arm(Cut::BeforeCommit);
    let failed = within(call(&b.chat, owner, "POST", &uri, None)).await;
    assert!(cut.fired());
    assert_eq!(error(&failed), (503, "chat_busy"), "{failed:?}");
    assert_eq!(keys_of(&pg, pending.id).await?, uploads);
    assert_eq!(
        listing(&b.chat, 1).await?.len(),
        4,
        "two uploads and two copies"
    );

    ok(reconcile::run(&b.chat, days_later(2), &mut Budget::new(50)).await)?;

    assert_eq!(listing(&b.chat, 1).await?, uploads);
    assert_eq!(
        statuses(&pg, &pending).await?,
        vec![Some("reserved".to_owned()); 2]
    );
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
#[ignore = "needs CHAT_TEST_S3_ENDPOINT (SeaweedFS)"]
async fn copies_a_late_commit_lands_on_stay_and_the_uploads_go(pg: PgPool) -> anyhow::Result<()> {
    let name = bucket(0).await?;
    let cut = PgCut::start(&pg).await;
    let b = bench(cut.pool.clone(), &[store(1, &name)], 1).await?;
    let owner = "landed-late";
    let chat_id = group(&pg).await?;
    let pending = reserve(&pg, owner, chat_id, 1, true).await?;
    put_objects(&b.chat, &pending).await?;
    let uploads = uploads_of(&pending);
    let uri = format!("/chat/media/{}/commit", pending.id);

    cut.arm(Cut::HoldCommit);
    let failed = within(call(&b.chat, owner, "POST", &uri, None)).await;
    assert!(cut.fired());
    assert_eq!(error(&failed), (503, "chat_busy"), "{failed:?}");
    cut.release();
    cut.landed().await;
    let kept = keys_of(&pg, pending.id).await?;
    assert!(kept.iter().all(|key| !uploads.contains(key)), "{kept:?}");

    ok(reconcile::run(&b.chat, days_later(2), &mut Budget::new(50)).await)?;

    assert_eq!(listing(&b.chat, 1).await?, kept);
    assert_eq!(
        statuses(&pg, &pending).await?,
        vec![Some("uploaded".to_owned()); 2]
    );
    assert_eq!(
        usage(&pg, owner).await?,
        Usage {
            bytes: pending.total(),
            reserved: 0,
            files: 1
        }
    );
    Ok(())
}
