use sqlx::PgPool;
use uuid::Uuid;

use super::library_sync::{request, state};

const COOLDOWN: u64 = 21600;

async fn jobs(pool: &PgPool) -> anyhow::Result<Vec<(String, i16)>> {
    Ok(
        sqlx::query_as("SELECT dedup_key, priority FROM background_jobs ORDER BY dedup_key")
            .fetch_all(pool)
            .await?,
    )
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn sync_now_reads_the_whole_library_first_and_then_waits_six_hours(
    pool: PgPool,
) -> anyhow::Result<()> {
    assert_eq!(state(&pool, "42", COOLDOWN).await?.status, "ready");

    let pressed = request(&pool, "soundcloud:users:42", COOLDOWN).await?;
    assert_eq!(pressed.status, "queued");
    let wait = pressed.available_at - chrono::Utc::now();
    assert!((21590..=21600).contains(&wait.num_seconds()));
    assert_eq!(
        jobs(&pool).await?,
        [
            ("liked-playlists:42:owner".into(), 50),
            ("liked-tracks:42:owner".into(), 50),
            ("owned-playlists:42:owner".into(), 50),
        ]
    );

    let again = request(&pool, "42", COOLDOWN).await?;
    assert_eq!(again.status, "cooldown");
    assert_eq!(again.available_at, pressed.available_at);
    assert_eq!(state(&pool, "42", COOLDOWN).await?.status, "cooldown");

    sqlx::query(
        "UPDATE library_sync_requests SET requested_at = now() - interval '6 hours 1 second'",
    )
    .execute(&pool)
    .await?;
    assert_eq!(state(&pool, "42", COOLDOWN).await?.status, "ready");
    assert_eq!(request(&pool, "42", COOLDOWN).await?.status, "queued");
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn sync_now_lifts_a_running_pass_without_sending_it_back_to_the_read_lane(
    pool: PgPool,
) -> anyhow::Result<()> {
    super::collection::sync_now(
        &pool,
        backend_contracts::CatalogCollection::LikedTracks,
        "42",
        20,
    )
    .await?;
    sqlx::query(
        "INSERT INTO catalog_collection_sync (subject_id, collection, scope, job_id, generation, snapshot_id, page_count, item_count)
         VALUES ('42', 'liked-tracks', 'owner', $1, 1, $2, 7, 1400)",
    )
    .bind(Uuid::now_v7())
    .bind(Uuid::now_v7())
    .execute(&pool)
    .await?;
    sqlx::query("UPDATE background_jobs SET priority = 20")
        .execute(&pool)
        .await?;

    request(&pool, "42", COOLDOWN).await?;

    let liked_tracks = jobs(&pool)
        .await?
        .into_iter()
        .find(|(key, _)| key == "liked-tracks:42:owner");
    assert_eq!(liked_tracks, Some(("liked-tracks:42:owner".into(), 30)));
    Ok(())
}
