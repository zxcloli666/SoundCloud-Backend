use std::time::Duration;

use sqlx::PgPool;

use super::tests::{
    TOTAL, copies_beside, counting, keys_of, left_for_reconcile, on_fake, sorted, uploaded,
};
use crate::modules::chat::fake_store::FakeStore;
use crate::modules::chat::media::ledger;
use crate::modules::chat::media::pg_cut::{Cut, PgCut, within};
use crate::modules::chat::media::rows::usage;
use crate::modules::chat::media::support::{IMAGE, THUMB, call, error, user};
use crate::modules::chat::media::wire::Picked;
use crate::modules::chat::sweeps::budget::Budget;
use crate::modules::chat::sweeps::media::delete;

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn a_commit_still_in_flight_at_the_re_read_keeps_the_copies_it_lands_on(pool: PgPool) {
    let recorder = counting();
    let _counting = metrics::set_default_local_recorder(&recorder);
    let fake = FakeStore::start(Duration::ZERO).await;
    let cut = PgCut::start(&pool).await;
    let chat = on_fake(&cut.pool, &fake);
    let owner = user();
    let (image, uploads) = uploaded(&pool, &fake, &owner).await;
    let uri = format!("/chat/media/{image}/commit");

    cut.arm(Cut::HoldCommit);
    let failed = within(call(&chat, &owner, "POST", &uri, None)).await;
    assert!(cut.fired());
    assert_eq!(error(&failed), (503, "chat_busy"), "{failed:?}");
    assert_eq!(keys_of(&pool, image).await, uploads);
    let copies = copies_beside(&fake, &uploads);
    assert_eq!(copies.len(), 2, "{:?}", fake.keys());
    assert_eq!(
        fake.keys(),
        sorted([uploads.clone(), copies.clone()].concat())
    );
    assert_eq!(
        left_for_reconcile(&recorder),
        2.0,
        "the copies, not the uploads"
    );

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
    assert_eq!(left_for_reconcile(&recorder), 2.0);
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn a_re_read_that_fails_keeps_every_object_and_counts_them_all(pool: PgPool) {
    let recorder = counting();
    let _counting = metrics::set_default_local_recorder(&recorder);
    let fake = FakeStore::start(Duration::ZERO).await;
    let cut = PgCut::start(&pool).await;
    let chat = on_fake(&cut.pool, &fake);
    let owner = user();
    let (image, uploads) = uploaded(&pool, &fake, &owner).await;
    let uri = format!("/chat/media/{image}/commit");

    cut.arm(Cut::HoldCommitUnreachable);
    let failed = within(call(&chat, &owner, "POST", &uri, None)).await;
    assert!(cut.fired());
    assert_eq!(error(&failed), (503, "chat_busy"), "{failed:?}");
    let copies = copies_beside(&fake, &uploads);
    assert_eq!(copies.len(), 2, "{:?}", fake.keys());
    assert_eq!(
        left_for_reconcile(&recorder),
        4.0,
        "nothing is known, all of it"
    );

    cut.release();
    cut.landed().await;
    let kept = keys_of(&pool, image).await;
    assert_eq!(sorted(kept.clone()), sorted(copies));
    assert_eq!(fake.keys(), sorted([kept, uploads].concat()));

    let retried = call(&chat, &owner, "POST", &uri, None).await;
    assert_eq!(retried.0, 200, "{retried:?}");
    assert_eq!(usage(&pool, &owner).await, (TOTAL, 0, 1));
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn a_re_read_that_finds_the_row_deleting_answers_busy_and_leaves_the_uploads(pool: PgPool) {
    let recorder = counting();
    let _counting = metrics::set_default_local_recorder(&recorder);
    let fake = FakeStore::start(Duration::ZERO).await;
    let cut = PgCut::start(&pool).await;
    let chat = on_fake(&cut.pool, &fake);
    let owner = user();
    let (image, uploads) = uploaded(&pool, &fake, &owner).await;
    let uri = format!("/chat/media/{image}/commit");

    cut.arm(Cut::HoldReply);
    let committing = {
        let (chat, owner, uri) = (chat.clone(), owner.clone(), uri.clone());
        tokio::spawn(async move { call(&chat, &owner, "POST", &uri, None).await })
    };
    cut.landed().await;
    let (deleted, _) = ledger::release(&chat.pg, &owner, &Picked::Ids(vec![image]))
        .await
        .expect("a user delete");
    assert_eq!(deleted.roots, 1);
    cut.release();
    let busy = within(committing).await.expect("a commit");

    assert!(cut.fired());
    assert_eq!(error(&busy), (503, "chat_busy"), "{busy:?}");
    let kept = keys_of(&pool, image).await;
    assert!(kept.iter().all(|key| !uploads.contains(key)), "{kept:?}");
    assert_eq!(fake.keys(), sorted([kept, uploads.clone()].concat()));
    assert_eq!(
        left_for_reconcile(&recorder),
        2.0,
        "the uploads, not the copies"
    );

    sqlx::query(
        "UPDATE chat.media SET restorable_until = now() - interval '1 second'
         WHERE id = $1 OR parent_id = $1",
    )
    .bind(image)
    .execute(&pool)
    .await
    .expect("a passed window");
    delete::run(&chat, &mut Budget::new(delete::CLAIMS))
        .await
        .expect("the delete step");
    let gone = call(&chat, &owner, "POST", &uri, None).await;

    assert_eq!(error(&gone), (404, "chat_not_found"), "{gone:?}");
    assert_eq!(
        fake.keys(),
        sorted(uploads),
        "the uploads wait for the reconcile"
    );
    assert_eq!(usage(&pool, &owner).await, (0, 0, 0));
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn a_commit_whose_row_is_gone_by_the_re_read_drops_every_copy(pool: PgPool) {
    let recorder = counting();
    let _counting = metrics::set_default_local_recorder(&recorder);
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
    let gone = within(committing).await.expect("a commit");

    assert!(cut.fired());
    assert_eq!(error(&gone), (404, "chat_not_found"), "{gone:?}");
    assert_eq!(fake.keys(), Vec::<String>::new());
    assert_eq!(left_for_reconcile(&recorder), 0.0);
}
