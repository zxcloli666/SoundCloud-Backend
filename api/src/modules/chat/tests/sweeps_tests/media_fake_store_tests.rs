use std::sync::Arc;
use std::time::Duration;

use axum::http::Method;
use rand::RngCore;
use redis::AsyncCommands;
use sqlx::PgPool;
use tokio::time::Instant;
use uuid::Uuid;

use super::support::{conversation, redis_db, user};
use crate::modules::chat::ChatState;
use crate::modules::chat::fake_store::{FakeStore, media_on};
use crate::modules::chat::media::rows::{Seed, seed, usage};
use crate::modules::chat::media::support::{IMAGE, THUMB, call, error, until_blocked};
use crate::modules::chat::moderation::testing::offline_redis;
use crate::modules::chat::sweeps::budget::Budget;
use crate::modules::chat::sweeps::media::fixtures::{Upload, ok, owner_delete, state_with, upload};
use crate::modules::chat::sweeps::media::mover::{self, Moved};
use crate::modules::chat::sweeps::media::{delete, expire};

const FAKE_DB: u8 = 3;

async fn bench(pg: &PgPool, stores: &[(u16, &FakeStore)]) -> anyhow::Result<Arc<ChatState>> {
    let redis = crate::redis::connect_url(&redis_db(FAKE_DB))?;
    let chat = state_with(pg.clone(), redis, media_on(stores));
    if let [(from, _), (to, _)] = stores {
        let mut conn = chat.redis.get().await?;
        let _: () = conn.del(mover::cursor_key(*from, *to)).await?;
    }
    Ok(chat)
}

async fn placed(
    pg: &PgPool,
    (id, store): (i16, &FakeStore),
    owner: &str,
    origin: Uuid,
    count: usize,
) -> anyhow::Result<Vec<Upload>> {
    let mut uploads = Vec::with_capacity(count);
    for _ in 0..count {
        let upload = upload(pg, owner, origin, id).await?;
        for (key, length) in upload.objects() {
            store.put(&key, random_bytes(length));
        }
        uploads.push(upload);
    }
    uploads.sort_by(|left, right| left.key.cmp(&right.key));
    Ok(uploads)
}

fn random_bytes(length: i64) -> Vec<u8> {
    let mut bytes = vec![0; usize::try_from(length).unwrap_or(0)];
    rand::thread_rng().fill_bytes(&mut bytes);
    bytes
}

async fn store_of(pg: &PgPool, id: Uuid) -> anyhow::Result<i16> {
    Ok(
        sqlx::query_scalar("SELECT store FROM chat.media WHERE id = $1")
            .bind(id)
            .fetch_one(pg)
            .await?,
    )
}

async fn cursor(chat: &ChatState, (from, to): (u16, u16)) -> anyhow::Result<Option<String>> {
    let mut conn = chat.redis.get().await?;
    Ok(conn.get(mover::cursor_key(from, to)).await?)
}

async fn move_all(chat: &ChatState, (from, to): (u16, u16)) -> anyhow::Result<u64> {
    ok(mover::run(chat, from, to, &mut Budget::new(mover::PER_RUN as u32)).await)
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
#[ignore = "requires a local Redis"]
async fn an_object_that_always_fails_is_skipped_and_retried_on_the_next_pass(
    pg: PgPool,
) -> anyhow::Result<()> {
    let (one, two) = (
        FakeStore::start(Duration::ZERO).await,
        FakeStore::start(Duration::ZERO).await,
    );
    let chat = bench(&pg, &[(1, &one), (2, &two)]).await?;
    let owner = user();
    let group = conversation(&pg, "group", &owner).await?;
    let uploads = placed(&pg, (1, &one), &owner, group, mover::PER_RUN as usize + 1).await?;
    let bad = &uploads[10];
    one.refuse(&bad.key);

    assert_eq!(move_all(&chat, (1, 2)).await?, mover::PER_RUN as u64 - 1);
    assert_eq!(
        cursor(&chat, (1, 2)).await?.as_deref(),
        Some(uploads[mover::PER_RUN as usize - 1].key.as_str()),
        "the run saved its cursor past the failing row"
    );
    assert_eq!(store_of(&pg, bad.id).await?, 1);
    assert_eq!(
        move_all(&chat, (1, 2)).await?,
        1,
        "the run after it goes on"
    );
    assert_eq!(cursor(&chat, (1, 2)).await?.as_deref(), Some(""));
    assert_eq!(
        move_all(&chat, (1, 2)).await?,
        0,
        "the next pass tries it again"
    );
    assert_eq!(store_of(&pg, bad.id).await?, 1);
    assert_eq!(
        one.asked(Method::GET)
            .iter()
            .filter(|key| **key == bad.key)
            .count(),
        2
    );

    one.allow(&bad.key);
    assert_eq!(move_all(&chat, (1, 2)).await?, 1);
    for upload in &uploads {
        assert_eq!(store_of(&pg, upload.id).await?, 2);
        for (key, _) in upload.objects() {
            assert!(two.object(&key).is_some(), "{key} is on store 2");
            assert!(one.object(&key).is_none(), "{key} left store 1");
        }
    }
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
#[ignore = "requires a local Redis"]
async fn a_slow_store_stops_the_delete_step_at_its_time_and_unclaims_the_rest(
    pg: PgPool,
) -> anyhow::Result<()> {
    let one = FakeStore::start(Duration::from_millis(200)).await;
    let chat = bench(&pg, &[(5, &one)]).await?;
    let owner = user();
    let group = conversation(&pg, "group", &owner).await?;
    let uploads = placed(&pg, (5, &one), &owner, group, 60).await?;
    for upload in &uploads {
        owner_delete(&pg, upload.id, -1).await?;
    }

    let started = Instant::now();
    let mut budget = Budget::until(delete::CLAIMS, started + Duration::from_millis(600));
    let removed = ok(delete::run(&chat, &mut budget).await)?;
    let took = started.elapsed();
    assert!(
        took < Duration::from_millis(1_500),
        "the step took {took:?}"
    );
    assert!(removed > 0 && removed < 60, "{removed} removed");
    let (left, leased): (i64, i64) = sqlx::query_as(
        "SELECT count(*), count(*) FILTER (WHERE lease_until IS NOT NULL)
         FROM chat.media WHERE status = 'deleting' AND kind <> 'thumb'",
    )
    .fetch_one(&pg)
    .await?;
    assert_eq!(left, 60 - removed as i64);
    assert_eq!(leased, 0, "rows it never tried go back unclaimed");
    assert!(one.asked(Method::DELETE).len() < 120);

    let mut finished = removed;
    let deadline = Instant::now() + Duration::from_secs(10);
    while finished < 60 {
        assert!(Instant::now() < deadline, "the next runs finish the rest");
        finished += ok(delete::run(&chat, &mut Budget::new(delete::CLAIMS)).await)?;
    }
    assert_eq!(finished, 60);
    for upload in &uploads {
        for (key, _) in upload.objects() {
            assert!(one.object(&key).is_none(), "{key} is gone");
        }
    }
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
#[ignore = "requires a local Redis"]
async fn a_slow_store_stops_the_mover_at_its_time_with_an_exact_cursor(
    pg: PgPool,
) -> anyhow::Result<()> {
    let (one, two) = (
        FakeStore::start(Duration::from_millis(50)).await,
        FakeStore::start(Duration::from_millis(50)).await,
    );
    let chat = bench(&pg, &[(3, &one), (4, &two)]).await?;
    let owner = user();
    let group = conversation(&pg, "group", &owner).await?;
    let uploads = placed(&pg, (3, &one), &owner, group, 12).await?;

    let started = Instant::now();
    let mut budget = Budget::until(mover::PER_RUN as u32, started + Duration::from_millis(100));
    let moved = ok(mover::run(&chat, 3, 4, &mut budget).await)?;
    let took = started.elapsed();
    assert!(took < Duration::from_secs(1), "the step took {took:?}");
    assert_eq!(moved, mover::PARALLEL as u64);
    assert_eq!(
        cursor(&chat, (3, 4)).await?.as_deref(),
        Some(uploads[mover::PARALLEL - 1].key.as_str())
    );
    for (at, upload) in uploads.iter().enumerate() {
        let expected = if at < mover::PARALLEL { 4 } else { 3 };
        assert_eq!(store_of(&pg, upload.id).await?, expected, "row {at}");
    }

    assert_eq!(move_all(&chat, (3, 4)).await?, 12 - mover::PARALLEL as u64);
    assert_eq!(cursor(&chat, (3, 4)).await?.as_deref(), Some(""));
    for upload in &uploads {
        assert_eq!(store_of(&pg, upload.id).await?, 4);
    }
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn the_mover_locks_the_root_before_it_touches_the_thumb(pg: PgPool) -> anyhow::Result<()> {
    let chat = state_with(pg.clone(), offline_redis(), Default::default());
    let (root, thumb) = (Uuid::new_v4(), Uuid::new_v4());
    let origin = Uuid::now_v7();
    for (id, kind, parent) in [(thumb, "thumb", Some(root)), (root, "image", None)] {
        sqlx::query(
            "INSERT INTO chat.media (id, store, object_key, owner_user, origin_conversation, kind, parent_id, bytes, status)
             VALUES ($1, 1, $2, 'owner', $3, $4, $5, 4096, 'lost')",
        )
        .bind(id)
        .bind(format!("m/{}", id.simple()))
        .bind(origin)
        .bind(kind)
        .bind(parent)
        .execute(&pg)
        .await?;
    }
    let mut owner = pg.begin().await?;
    sqlx::query("SELECT id FROM chat.media WHERE id = $1 FOR UPDATE")
        .bind(root)
        .execute(&mut *owner)
        .await?;
    let moving = {
        let chat = chat.clone();
        tokio::spawn(async move { mover::move_one(&chat, root, true, 1, 2).await })
    };
    until_blocked(&pg).await;
    sqlx::query("SELECT id FROM chat.media WHERE id = $1 FOR UPDATE NOWAIT")
        .bind(thumb)
        .execute(&mut *owner)
        .await
        .expect("the waiting mover holds no lock on the thumb");
    owner.commit().await?;
    assert_eq!(ok(moving.await?)?, Moved::Yes);
    assert_eq!(store_of(&pg, root).await?, 2);
    assert_eq!(store_of(&pg, thumb).await?, 2);
    Ok(())
}

async fn row_keys(pg: &PgPool, root: Uuid) -> anyhow::Result<Vec<String>> {
    Ok(sqlx::query_scalar(
        "SELECT object_key FROM chat.media WHERE id = $1 OR parent_id = $1 ORDER BY kind",
    )
    .bind(root)
    .fetch_all(pg)
    .await?)
}

fn sorted(mut keys: Vec<String>) -> Vec<String> {
    keys.sort();
    keys
}

async fn put_uploads(pg: &PgPool, store: &FakeStore, owner: &str) -> anyhow::Result<Uuid> {
    let group = conversation(pg, "group", owner).await?;
    let (image, _) = seed(pg, Seed::image(owner, group, "reserved")).await;
    let uploads = row_keys(pg, image).await?;
    store.put(&uploads[0], vec![1; IMAGE as usize]);
    store.put(&uploads[1], vec![2; THUMB as usize]);
    Ok(image)
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
#[ignore = "requires a local Redis"]
async fn the_mover_and_the_delete_sweep_follow_the_kept_key_not_a_later_put(
    pg: PgPool,
) -> anyhow::Result<()> {
    let (one, two) = (
        FakeStore::start(Duration::ZERO).await,
        FakeStore::start(Duration::ZERO).await,
    );
    let chat = bench(&pg, &[(1, &one), (2, &two)]).await?;
    let owner = user();
    let image = put_uploads(&pg, &one, &owner).await?;
    let uploads = row_keys(&pg, image).await?;
    let commit = call(
        &chat,
        &owner,
        "POST",
        &format!("/chat/media/{image}/commit"),
        None,
    )
    .await;
    assert_eq!(commit.0, 200, "{commit:?}");
    let kept = row_keys(&pg, image).await?;
    one.put(&uploads[0], vec![9; IMAGE as usize]);

    assert_eq!(
        ok(mover::move_one(&chat, image, false, 1, 2).await)?,
        Moved::Yes
    );
    assert_eq!(two.keys(), sorted(kept.clone()));
    assert_eq!(two.object(&kept[0]), Some(vec![1; IMAGE as usize]));
    assert_eq!(two.object(&kept[1]), Some(vec![2; THUMB as usize]));
    assert_eq!(one.keys(), [uploads[0].clone()]);

    assert!(owner_delete(&pg, image, -1).await?);
    ok(delete::run(&chat, &mut Budget::new(delete::CLAIMS)).await)?;
    assert!(two.keys().is_empty());
    assert_eq!(usage(&pg, &owner).await, (0, 0, 0));
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
#[ignore = "requires a local Redis"]
async fn a_failed_copy_left_alone_expires_with_its_upload_objects(
    pg: PgPool,
) -> anyhow::Result<()> {
    let one = FakeStore::start(Duration::ZERO).await;
    let chat = bench(&pg, &[(1, &one)]).await?;
    let owner = user();
    let image = put_uploads(&pg, &one, &owner).await?;
    let uploads = row_keys(&pg, image).await?;
    one.fail_copies(true);
    let failed = call(
        &chat,
        &owner,
        "POST",
        &format!("/chat/media/{image}/commit"),
        None,
    )
    .await;
    assert_eq!(error(&failed), (503, "chat_busy"), "{failed:?}");
    assert_eq!(one.keys(), sorted(uploads));

    sqlx::query(
        "UPDATE chat.media SET delete_after = now() - interval '1 second' WHERE id = $1 OR parent_id = $1",
    )
    .bind(image)
    .execute(&pg)
    .await?;
    ok(expire::run(&chat, &mut Budget::new(expire::BATCHES)).await)?;
    ok(delete::run(&chat, &mut Budget::new(delete::CLAIMS)).await)?;
    assert!(one.keys().is_empty());
    assert_eq!(usage(&pg, &owner).await, (0, 0, 0));
    Ok(())
}
