use sqlx::PgPool;
use uuid::Uuid;

use super::{StoreSpec, bench, bucket, present, put_objects, store};
use crate::modules::chat::sweeps::budget::Budget;
use crate::modules::chat::sweeps::media::fixtures::{
    Usage, attach, ok, owner_delete, pass_window, snapshot, status, upload, usage,
};
use crate::modules::chat::sweeps::media::{delete, gauges, withdrawn};

async fn group(pg: &PgPool) -> anyhow::Result<Uuid> {
    let id = Uuid::now_v7();
    sqlx::query("INSERT INTO chat.conversations (id, kind, created_by) VALUES ($1, 'group', 'x')")
        .bind(id)
        .execute(pg)
        .await?;
    Ok(id)
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
#[ignore = "needs CHAT_TEST_S3_ENDPOINT (SeaweedFS)"]
async fn deleting_rows_lose_every_object_then_the_row_and_a_missing_key_counts_as_done(
    pg: PgPool,
) -> anyhow::Result<()> {
    let name = bucket(0).await?;
    let b = bench(pg.clone(), &[store(1, &name)], 1).await?;
    let owner = "deleter";
    let chat_id = group(&pg).await?;
    let mut doomed = Vec::new();
    for index in 0..5 {
        let image = upload(&pg, owner, chat_id, 1).await?;
        if index > 0 {
            put_objects(&b.chat, &image).await?;
        }
        attach(&pg, image.id, chat_id, index + 1).await?;
        owner_delete(&pg, image.id, 15).await?;
        pass_window(&pg, image.id).await?;
        doomed.push(image);
    }
    let kept = upload(&pg, owner, chat_id, 1).await?;
    put_objects(&b.chat, &kept).await?;
    attach(&pg, kept.id, chat_id, 9).await?;
    let open = upload(&pg, owner, chat_id, 1).await?;
    put_objects(&b.chat, &open).await?;
    attach(&pg, open.id, chat_id, 10).await?;
    owner_delete(&pg, open.id, 15).await?;

    assert_eq!(
        ok(delete::run(&b.chat, &mut Budget::new(4)).await)?,
        0,
        "refs still hold them"
    );
    ok(withdrawn::run(&b.chat, &mut Budget::new(4)).await)?;
    assert_eq!(ok(delete::run(&b.chat, &mut Budget::new(4)).await)?, 5);

    for image in &doomed {
        assert_eq!(status(&pg, image.id).await?, None);
        assert_eq!(present(&b.chat, image).await?, vec![false, false]);
    }
    assert_eq!(present(&b.chat, &kept).await?, vec![true, true]);
    assert_eq!(
        present(&b.chat, &open).await?,
        vec![true, true],
        "the restore window is open"
    );
    assert_eq!(status(&pg, open.id).await?.as_deref(), Some("deleting"));
    assert_eq!(
        usage(&pg, owner).await?,
        Usage {
            bytes: kept.total(),
            reserved: 0,
            files: 1
        }
    );
    let before = snapshot(&pg).await?;
    assert_eq!(ok(delete::run(&b.chat, &mut Budget::new(4)).await)?, 0);
    assert_eq!(snapshot(&pg).await?, before);
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
#[ignore = "needs CHAT_TEST_S3_ENDPOINT (SeaweedFS)"]
async fn a_failing_store_keeps_the_lease_and_ten_attempts_count_as_stuck(
    pg: PgPool,
) -> anyhow::Result<()> {
    let name = bucket(0).await?;
    let dead = StoreSpec {
        id: 3,
        bucket: &name,
        endpoint: Some("http://127.0.0.1:1"),
    };
    let b = bench(pg.clone(), &[store(1, &name), dead], 1).await?;
    let chat_id = group(&pg).await?;
    let image = upload(&pg, "stuck", chat_id, 3).await?;
    owner_delete(&pg, image.id, -1).await?;

    assert_eq!(ok(delete::run(&b.chat, &mut Budget::new(4)).await)?, 0);
    let (attempts, leased): (i16, bool) =
        sqlx::query_as("SELECT attempts, lease_until > now() FROM chat.media WHERE id = $1")
            .bind(image.id)
            .fetch_one(&pg)
            .await?;
    assert_eq!((attempts, leased), (1, true));
    assert_eq!(ok(delete::run(&b.chat, &mut Budget::new(4)).await)?, 0);
    let still: i16 = sqlx::query_scalar("SELECT attempts FROM chat.media WHERE id = $1")
        .bind(image.id)
        .fetch_one(&pg)
        .await?;
    assert_eq!(still, 1, "a held lease is not claimed again");

    sqlx::query("UPDATE chat.media SET attempts = 9, lease_until = now() - interval '1 second' WHERE id = $1")
        .bind(image.id)
        .execute(&pg)
        .await?;
    assert_eq!(ok(gauges::stuck(&b.chat).await)?, 0);
    ok(delete::run(&b.chat, &mut Budget::new(4)).await)?;
    assert_eq!(ok(gauges::stuck(&b.chat).await)?, 1);
    assert_eq!(status(&pg, image.id).await?.as_deref(), Some("deleting"));
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
#[ignore = "needs CHAT_TEST_S3_ENDPOINT (SeaweedFS)"]
async fn a_crash_in_the_middle_of_the_deletes_ends_the_same_without_a_second_release(
    pg: PgPool,
) -> anyhow::Result<()> {
    let name = bucket(0).await?;
    let b = bench(pg.clone(), &[store(1, &name)], 1).await?;
    let owner = "crasher";
    let chat_id = group(&pg).await?;
    let survivor = upload(&pg, owner, chat_id, 1).await?;
    put_objects(&b.chat, &survivor).await?;
    attach(&pg, survivor.id, chat_id, 1).await?;
    let mut doomed = Vec::new();
    for _ in 0..6 {
        let image = upload(&pg, owner, chat_id, 1).await?;
        put_objects(&b.chat, &image).await?;
        owner_delete(&pg, image.id, -1).await?;
        doomed.push(image);
    }
    let charged = usage(&pg, owner).await?;
    sqlx::query(
        "UPDATE chat.media SET lease_until = now() + interval '5 minutes', attempts = 1
         WHERE status = 'deleting' AND kind <> 'thumb'",
    )
    .execute(&pg)
    .await?;
    for image in doomed.iter().take(3) {
        for (key, _) in image.objects() {
            ok(b.chat.s3.delete(1, &key).await)?;
        }
    }

    assert_eq!(
        ok(delete::run(&b.chat, &mut Budget::new(4)).await)?,
        0,
        "the dead run still holds the lease"
    );
    sqlx::query(
        "UPDATE chat.media SET lease_until = now() - interval '1 second' WHERE status = 'deleting'",
    )
    .execute(&pg)
    .await?;
    assert_eq!(ok(delete::run(&b.chat, &mut Budget::new(4)).await)?, 6);

    for image in &doomed {
        assert_eq!(status(&pg, image.id).await?, None);
        assert_eq!(present(&b.chat, image).await?, vec![false, false]);
    }
    assert_eq!(usage(&pg, owner).await?, charged);
    assert_eq!(charged.bytes, survivor.total());
    Ok(())
}
