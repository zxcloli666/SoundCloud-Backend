use sha2::{Digest, Sha256};
use sqlx::PgPool;
use uuid::Uuid;

use super::{bench, bucket, ciphertext, days_later, exists, listing, store};
use crate::modules::chat::ChatState;
use crate::modules::chat::sweeps::budget::Budget;
use crate::modules::chat::sweeps::media::fixtures::{Upload, attach, ok, status, upload, usage};
use crate::modules::chat::sweeps::media::{mover, reconcile};

async fn group(pg: &PgPool) -> anyhow::Result<Uuid> {
    let id = Uuid::now_v7();
    sqlx::query("INSERT INTO chat.conversations (id, kind, created_by) VALUES ($1, 'group', 'x')")
        .bind(id)
        .execute(pg)
        .await?;
    Ok(id)
}

async fn placed(chat: &ChatState, upload: &Upload) -> anyhow::Result<Vec<[u8; 32]>> {
    let store = u16::try_from(upload.store)?;
    let mut digests = Vec::new();
    for (key, length) in upload.objects() {
        let bytes = ciphertext(length);
        digests.push(Sha256::digest(&bytes).into());
        ok(chat.s3.put(store, &key, bytes).await)?;
    }
    Ok(digests)
}

async fn digests_on(
    chat: &ChatState,
    store: u16,
    upload: &Upload,
) -> anyhow::Result<Vec<[u8; 32]>> {
    let mut digests = Vec::new();
    for (key, _) in upload.objects() {
        let bytes = ok(chat.s3.get(store, &key).await)?.expect("an object");
        digests.push(Sha256::digest(&bytes).into());
    }
    Ok(digests)
}

async fn store_of(pg: &PgPool, id: Uuid) -> anyhow::Result<i16> {
    Ok(
        sqlx::query_scalar("SELECT store FROM chat.media WHERE id = $1")
            .bind(id)
            .fetch_one(pg)
            .await?,
    )
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
#[ignore = "needs CHAT_TEST_S3_ENDPOINT (SeaweedFS)"]
async fn the_mover_copies_checks_flips_and_deletes_and_reconcile_clears_crash_leftovers(
    pg: PgPool,
) -> anyhow::Result<()> {
    let (one, two) = (bucket(0).await?, bucket(1).await?);
    let b = bench(pg.clone(), &[store(1, &one), store(2, &two)], 2).await?;
    let chat_id = group(&pg).await?;
    let owner = "mover";
    let mut old = Vec::new();
    for seq in 1..=5 {
        let image = upload(&pg, owner, chat_id, 1).await?;
        let digests = placed(&b.chat, &image).await?;
        attach(&pg, image.id, chat_id, seq).await?;
        old.push((image, digests));
    }
    let fresh = upload(&pg, owner, chat_id, 2).await?;
    let fresh_digests = placed(&b.chat, &fresh).await?;
    attach(&pg, fresh.id, chat_id, 9).await?;
    let charged = usage(&pg, owner).await?;
    let (crashed_after_flip, crashed_digests) = old.pop().expect("an image");
    for (key, _) in crashed_after_flip.objects() {
        let bytes = ok(b.chat.s3.get(1, &key).await)?.expect("an object");
        ok(b.chat.s3.put(2, &key, bytes).await)?;
    }
    sqlx::query("UPDATE chat.media SET store = 2 WHERE id = $1 OR parent_id = $1")
        .bind(crashed_after_flip.id)
        .execute(&pg)
        .await?;
    let (crashed_before_flip, _) = old.pop().expect("an image");
    for (key, _) in crashed_before_flip.objects() {
        ok(b.chat.s3.put(2, &key, ciphertext(4_096)).await)?;
    }

    assert_eq!(ok(mover::run(&b.chat, 1, 2).await)?, 4);

    for (image, digests) in &old {
        assert_eq!(store_of(&pg, image.id).await?, 2);
        assert_eq!(&digests_on(&b.chat, 2, image).await?, digests);
        for (key, _) in image.objects() {
            assert!(!exists(&b.chat, 1, &key).await?);
        }
    }
    assert_eq!(
        store_of(&pg, crashed_before_flip.id).await?,
        2,
        "the next lap moves it"
    );
    assert_eq!(digests_on(&b.chat, 2, &fresh).await?, fresh_digests);
    assert_eq!(store_of(&pg, fresh.id).await?, 2);
    assert_eq!(usage(&pg, owner).await?, charged, "moving charges nothing");
    let leftovers: Vec<String> = crashed_after_flip
        .objects()
        .into_iter()
        .map(|(key, _)| key)
        .collect();
    let on_one = listing(&b.chat, 1).await?;
    assert!(
        leftovers.iter().all(|key| on_one.contains(key)),
        "the crash left the old copy"
    );

    ok(reconcile::run(&b.chat, days_later(2), &mut Budget::new(50)).await)?;
    assert!(
        listing(&b.chat, 1).await?.is_empty(),
        "reconcile removed the leftover"
    );
    assert_eq!(
        status(&pg, crashed_after_flip.id).await?.as_deref(),
        Some("attached")
    );
    assert_eq!(
        digests_on(&b.chat, 2, &crashed_after_flip).await?,
        crashed_digests
    );
    assert_eq!(ok(mover::run(&b.chat, 1, 2).await)?, 0);
    Ok(())
}
