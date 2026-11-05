use sqlx::PgPool;
use uuid::Uuid;

use super::{
    bench, bucket, ciphertext, days_later, exists, later_today, listing, put_objects, store,
};
use crate::modules::chat::media::support::call;
use crate::modules::chat::sweeps::budget::Budget;
use crate::modules::chat::sweeps::media::fixtures::{
    Usage, attach, object_key, ok, reserve, snapshot, status, statuses, upload, usage,
};
use crate::modules::chat::sweeps::media::reconcile::{self, Stage, cursor};

pub(super) async fn group(pg: &PgPool) -> anyhow::Result<Uuid> {
    let id = Uuid::now_v7();
    sqlx::query("INSERT INTO chat.conversations (id, kind, created_by) VALUES ($1, 'group', 'x')")
        .bind(id)
        .execute(pg)
        .await?;
    Ok(id)
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
#[ignore = "needs CHAT_TEST_S3_ENDPOINT (SeaweedFS)"]
async fn reconcile_deletes_old_orphans_and_marks_rows_without_objects_lost(
    pg: PgPool,
) -> anyhow::Result<()> {
    let name = bucket(0).await?;
    let b = bench(pg.clone(), &[store(1, &name)], 1).await?;
    let owner = "reconciled";
    let chat_id = group(&pg).await?;
    let healthy = upload(&pg, owner, chat_id, 1).await?;
    put_objects(&b.chat, &healthy).await?;
    attach(&pg, healthy.id, chat_id, 1).await?;
    let vanished = upload(&pg, owner, chat_id, 1).await?;
    put_objects(&b.chat, &vanished).await?;
    attach(&pg, vanished.id, chat_id, 2).await?;
    let unsent = upload(&pg, owner, chat_id, 1).await?;
    let pending = reserve(&pg, owner, chat_id, 1, true).await?;
    let orphan = object_key();
    ok(b.chat.s3.put(1, &orphan, ciphertext(4_096)).await)?;
    for (key, _) in vanished.objects() {
        ok(b.chat.s3.delete(1, &key).await)?;
    }

    let found = ok(reconcile::run(&b.chat, later_today(), &mut Budget::new(50)).await)?;

    assert_eq!(found, 2 + 1, "two rows lost and one usage row recounted");
    assert!(exists(&b.chat, 1, &orphan).await?, "a young orphan stays");
    assert_eq!(
        statuses(&pg, &vanished).await?,
        vec![Some("lost".to_owned()); 2]
    );
    assert_eq!(
        statuses(&pg, &unsent).await?,
        vec![Some("deleting".to_owned()); 2],
        "lost without refs is freed"
    );
    assert_eq!(status(&pg, healthy.id).await?.as_deref(), Some("attached"));
    assert_eq!(status(&pg, pending.id).await?.as_deref(), Some("reserved"));
    assert_eq!(
        usage(&pg, owner).await?,
        Usage {
            bytes: healthy.total(),
            reserved: pending.total(),
            files: 1
        }
    );
    let saved = ok(cursor::load(&b.chat).await)?.expect("a cursor");
    assert_eq!((saved.stage, saved.drift), (Stage::Done, 0));
    let before = snapshot(&pg).await?;
    assert_eq!(
        ok(reconcile::run(&b.chat, later_today(), &mut Budget::new(50)).await)?,
        0,
        "once a night"
    );
    assert_eq!(snapshot(&pg).await?, before);

    ok(reconcile::run(&b.chat, days_later(2), &mut Budget::new(50)).await)?;
    assert!(
        !exists(&b.chat, 1, &orphan).await?,
        "an orphan older than 24 h goes"
    );
    assert!(exists(&b.chat, 1, &healthy.key).await?);
    assert!(!listing(&b.chat, 1).await?.contains(&orphan));
    assert_eq!(ok(cursor::load(&b.chat).await)?.expect("a cursor").drift, 0);
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
#[ignore = "needs CHAT_TEST_S3_ENDPOINT (SeaweedFS)"]
async fn a_reconcile_cut_by_a_restart_resumes_from_the_redis_cursor(
    pg: PgPool,
) -> anyhow::Result<()> {
    let name = bucket(0).await?;
    let b = bench(pg.clone(), &[store(1, &name)], 1).await?;
    let chat_id = group(&pg).await?;
    let owner = "resumed";
    let mut images = Vec::new();
    for seq in 1..=3 {
        let image = upload(&pg, owner, chat_id, 1).await?;
        put_objects(&b.chat, &image).await?;
        attach(&pg, image.id, chat_id, seq).await?;
        images.push(image);
    }
    let gone = upload(&pg, owner, chat_id, 1).await?;
    attach(&pg, gone.id, chat_id, 4).await?;
    sqlx::query("UPDATE chat.usage SET bytes = bytes + 4096 WHERE user_id = $1")
        .bind(owner)
        .execute(&pg)
        .await?;

    ok(reconcile::run(&b.chat, later_today(), &mut Budget::new(1)).await)?;
    let saved = ok(cursor::load(&b.chat).await)?.expect("a cursor");
    assert_eq!(saved.stage, Stage::Tail);
    assert!(!saved.after.is_empty());

    let restarted = bench_again(&b).await;
    ok(reconcile::run(&restarted, later_today(), &mut Budget::new(50)).await)?;
    let done = ok(cursor::load(&restarted).await)?.expect("a cursor");
    assert_eq!(done.stage, Stage::Done);
    assert_eq!(done.drift, 4_096);
    assert_eq!(status(&pg, gone.id).await?.as_deref(), Some("lost"));
    let expected: i64 = images.iter().map(|image| image.total()).sum();
    assert_eq!(usage(&pg, owner).await?.bytes, expected);
    Ok(())
}

async fn bench_again(b: &super::Bench) -> std::sync::Arc<crate::modules::chat::ChatState> {
    let media = b.chat.config.media.clone();
    crate::modules::chat::sweeps::media::fixtures::state_with(
        b.pg.clone(),
        b.chat.redis.clone(),
        media,
    )
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
#[ignore = "needs CHAT_TEST_S3_ENDPOINT (SeaweedFS)"]
async fn a_store_that_lists_nothing_while_rows_live_there_is_left_alone(
    pg: PgPool,
) -> anyhow::Result<()> {
    let name = bucket(0).await?;
    let b = bench(pg.clone(), &[store(1, &name)], 1).await?;
    let chat_id = group(&pg).await?;
    let owner = "wiped";
    let image = upload(&pg, owner, chat_id, 1).await?;
    attach(&pg, image.id, chat_id, 1).await?;
    let charged = usage(&pg, owner).await?;

    ok(reconcile::run(&b.chat, later_today(), &mut Budget::new(50)).await)?;

    assert_eq!(status(&pg, image.id).await?.as_deref(), Some("attached"));
    assert_eq!(usage(&pg, owner).await?, charged);
    let saved = ok(cursor::load(&b.chat).await)?.expect("a cursor");
    assert_eq!(saved.stage, Stage::Done);
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
#[ignore = "needs CHAT_TEST_S3_ENDPOINT (SeaweedFS)"]
async fn a_put_after_commit_is_an_orphan_and_the_kept_copy_stays_live(
    pg: PgPool,
) -> anyhow::Result<()> {
    let name = bucket(0).await?;
    let b = bench(pg.clone(), &[store(1, &name)], 1).await?;
    let owner = "recommitted";
    let chat_id = group(&pg).await?;
    let pending = reserve(&pg, owner, chat_id, 1, true).await?;
    put_objects(&b.chat, &pending).await?;
    let commit = call(
        &b.chat,
        owner,
        "POST",
        &format!("/chat/media/{}/commit", pending.id),
        None,
    )
    .await;
    assert_eq!(commit.0, 200, "{commit:?}");
    let kept: Vec<String> = sqlx::query_scalar(
        "SELECT object_key FROM chat.media WHERE id = $1 OR parent_id = $1 ORDER BY kind",
    )
    .bind(pending.id)
    .fetch_all(&pg)
    .await?;
    assert!(!kept.contains(&pending.key));
    ok(b.chat
        .s3
        .put(1, &pending.key, ciphertext(pending.bytes))
        .await)?;

    ok(reconcile::run(&b.chat, days_later(2), &mut Budget::new(50)).await)?;

    assert!(
        !exists(&b.chat, 1, &pending.key).await?,
        "the late put is an orphan"
    );
    for key in &kept {
        assert!(exists(&b.chat, 1, key).await?, "{key} stays");
    }
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
