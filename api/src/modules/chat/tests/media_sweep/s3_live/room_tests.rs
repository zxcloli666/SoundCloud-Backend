use sqlx::PgPool;
use uuid::Uuid;

use super::{Bench, bench, bucket, present, put_objects, store};
use crate::modules::chat::ChatState;
use crate::modules::chat::sweeps::budget::Budget;
use crate::modules::chat::sweeps::media::fixtures::{
    Upload, attach, handle, ok, report, reserve, rows_of, snapshot, upload, usage,
};
use crate::modules::chat::sweeps::media::{self, delete, holds, reconcile, withdrawn};
use crate::modules::chat::sweeps::purge;
use crate::modules::chat::sweeps::tests::support::{messages, room};

async fn conversation(pg: &PgPool, kind: &str) -> anyhow::Result<Uuid> {
    let id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO chat.conversations (id, kind, created_by, room_alive_until)
         VALUES ($1, $2, 'x', CASE WHEN $2 = 'room' THEN now() + interval '10 minutes' END)",
    )
    .bind(id)
    .bind(kind)
    .execute(pg)
    .await?;
    Ok(id)
}

async fn settle_all(chat: &ChatState, purge_budget: u32) -> anyhow::Result<()> {
    let mut left = purge_budget;
    for _ in 0..4 {
        if left > 0 {
            ok(purge::run_once(chat).await)?;
            left -= 1;
        }
        ok(media::expire::run(chat, &mut Budget::new(5)).await)?;
        ok(withdrawn::run(chat, &mut Budget::new(5)).await)?;
        ok(holds::run(chat, &mut Budget::new(5)).await)?;
        ok(delete::run(chat, &mut Budget::new(5)).await)?;
    }
    Ok(())
}

async fn all_gone(b: &Bench, images: &[Upload]) -> anyhow::Result<bool> {
    for image in images {
        if present(&b.chat, image).await?.contains(&true) {
            return Ok(false);
        }
    }
    Ok(true)
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
#[ignore = "needs CHAT_TEST_S3_ENDPOINT (SeaweedFS)"]
async fn an_ended_room_leaves_no_row_no_object_and_no_charge_except_a_reported_image(
    pg: PgPool,
) -> anyhow::Result<()> {
    let name = bucket(0).await?;
    let b = bench(pg.clone(), &[store(1, &name)], 1).await?;
    let senders = ["star-a", "star-b", "star-c"];
    let elsewhere = conversation(&pg, "group").await?;
    let mut before = Vec::new();
    for sender in senders {
        let keep = upload(&pg, sender, elsewhere, 1).await?;
        put_objects(&b.chat, &keep).await?;
        attach(&pg, keep.id, elsewhere, 1).await?;
        before.push(usage(&pg, sender).await?);
    }
    let room_chat = conversation(&pg, "room").await?;
    let mut images = Vec::new();
    let mut seq = 0;
    for sender in senders {
        for _ in 0..10 {
            seq += 1;
            let image = upload(&pg, sender, room_chat, 1).await?;
            put_objects(&b.chat, &image).await?;
            messages(&pg, room_chat, seq..=seq, None).await?;
            attach(&pg, image.id, room_chat, seq).await?;
            images.push(image);
        }
    }
    let unsent = upload(&pg, senders[1], room_chat, 1).await?;
    put_objects(&b.chat, &unsent).await?;
    let half_done = reserve(&pg, senders[2], room_chat, 1, true).await?;
    put_objects(&b.chat, &half_done).await?;
    images.push(unsent);
    images.push(half_done);
    let reported = images[7].clone();
    let case = report(&pg, room_chat, &[reported.id]).await?;
    let live = room(&b.chat, senders[0], room_chat).await?;

    assert!(
        b.chat
            .rooms
            .end(&live.code)
            .await
            .map_err(|error| anyhow::anyhow!("{error:?}"))?
    );
    ok(purge::conversation(&b.chat, room_chat, &mut Budget::new(1)).await)?;
    let restarted = crate::modules::chat::sweeps::media::fixtures::state_with(
        pg.clone(),
        b.chat.redis.clone(),
        b.chat.config.media.clone(),
    );
    settle_all(&restarted, 4).await?;

    assert_eq!(
        rows_of(&pg, room_chat).await?,
        2,
        "only the reported image and its thumb"
    );
    let others: Vec<Upload> = images
        .iter()
        .filter(|image| image.id != reported.id)
        .cloned()
        .collect();
    assert!(all_gone(&b, &others).await?);
    assert_eq!(present(&b.chat, &reported).await?, vec![true, true]);
    for (sender, was) in senders.iter().zip(&before) {
        assert_eq!(usage(&pg, sender).await?, *was);
    }
    let quiet = snapshot(&pg).await?;
    settle_all(&restarted, 4).await?;
    assert_eq!(snapshot(&pg).await?, quiet, "a rerun changes nothing");

    handle(&pg, case).await?;
    settle_all(&restarted, 0).await?;
    assert_eq!(rows_of(&pg, room_chat).await?, 0);
    assert!(all_gone(&b, &images).await?);
    ok(reconcile::run(&restarted, super::later_today(), &mut Budget::new(50)).await)?;
    let drift = ok(reconcile::cursor::load(&restarted).await)?
        .expect("a cursor")
        .drift;
    assert_eq!(drift, 0);
    Ok(())
}
