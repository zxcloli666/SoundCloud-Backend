use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use backend_contracts::{AdminMaintenancePayload, JobKind};
use sqlx::PgPool;
use uuid::Uuid;

use super::super::TRACK_WORK_KEYS;
use super::super::tests::handler;
use crate::queue::{ClaimOrder, JobRepository, NewJob};

const DRIFT: &str = "SELECT count(*)
    FROM tracks AS track
    FULL JOIN track_work_keys AS keys ON keys.track_id = track.id
    WHERE track.id IS NULL
       OR keys.track_id IS NULL
       OR (track.primary_artist_id, track.superseded_by IS NOT NULL, track.work_key,
           track.recording_key, track.work_normalizer_version)
          IS DISTINCT FROM
          (keys.primary_artist_id, keys.superseded, keys.work_key, keys.recording_key,
           keys.work_normalizer_version)";

const STORE_KEYS: &str = "WITH updated AS (
        UPDATE tracks
        SET work_key = 'w' || sc_track_id,
            recording_key = 'r' || sc_track_id,
            work_normalizer_version = 1
        WHERE sc_track_id = 's' || (1 + floor(random() * 30000))::int
        RETURNING id, primary_artist_id, superseded_by IS NOT NULL AS superseded,
                  work_key, recording_key
    )
    INSERT INTO track_work_keys AS stored (
        track_id, primary_artist_id, superseded, work_key, recording_key, work_normalizer_version
    )
    SELECT id, primary_artist_id, superseded, work_key, recording_key, 1 FROM updated
    ORDER BY id
    ON CONFLICT (track_id) DO UPDATE
    SET work_key = EXCLUDED.work_key,
        recording_key = EXCLUDED.recording_key,
        work_normalizer_version = EXCLUDED.work_normalizer_version";

const MOVE_ARTIST: &str = "UPDATE tracks
    SET primary_artist_id = (SELECT id FROM artists ORDER BY random() LIMIT 1)
    WHERE sc_track_id = 's' || (1 + floor(random() * 30000))::int";

const INSERT: &str = "INSERT INTO tracks (
        sc_track_id, urn, title, title_normalized, duration_ms, sharing
    )
    VALUES ('s' || $1::int, 'soundcloud:tracks:s' || $1::int, 'Track', 'track', 180000, 'public')";

#[derive(Debug, sqlx::FromRow)]
struct Totals {
    status: String,
    scanned: i64,
    changed: i64,
    merged: i64,
    skipped: i64,
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn the_triggers_mirror_the_artist_and_the_superseded_flag(
    pool: PgPool,
) -> anyhow::Result<()> {
    let artists = seed_artists(&pool, 2).await?;
    seed(&pool, 1, 2).await?;
    let first = track_id(&pool, 1).await?;
    let second = track_id(&pool, 2).await?;

    assert_eq!(mirror(&pool, first).await?, (None, false, None, None));

    set_artist(&pool, first, artists[0]).await?;
    assert_eq!(
        mirror(&pool, first).await?,
        (Some(artists[0]), false, None, None)
    );

    set_artist(&pool, first, artists[1]).await?;
    assert_eq!(
        mirror(&pool, first).await?,
        (Some(artists[1]), false, None, None)
    );

    sqlx::query("UPDATE tracks SET superseded_by = $2 WHERE id = $1")
        .bind(first)
        .bind(second)
        .execute(&pool)
        .await?;
    assert_eq!(
        mirror(&pool, first).await?,
        (Some(artists[1]), true, None, None)
    );

    let before = row_version(&pool, first).await?;
    sqlx::query("UPDATE tracks SET title = 'Other title' WHERE id = $1")
        .bind(first)
        .execute(&pool)
        .await?;
    assert_eq!(row_version(&pool, first).await?, before);

    sqlx::query("DELETE FROM tracks WHERE id = $1")
        .bind(first)
        .execute(&pool)
        .await?;
    assert_eq!(rows_of(&pool, first).await?, 0);
    assert_eq!(rows_of(&pool, second).await?, 1);
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn the_backfill_and_the_check_repair_keys_missing_rows_and_mirrors(
    pool: PgPool,
) -> anyhow::Result<()> {
    seed(&pool, 1, 12000).await?;
    execute(
        &pool,
        "UPDATE tracks
         SET work_key = 'w' || sc_track_id,
             recording_key = 'r' || sc_track_id,
             work_normalizer_version = 1
         WHERE substr(sc_track_id, 2)::int <= 3000",
    )
    .await?;
    execute(
        &pool,
        "DELETE FROM track_work_keys
         WHERE track_id IN (
             SELECT id FROM tracks WHERE substr(sc_track_id, 2)::int BETWEEN 2951 AND 3050
         )",
    )
    .await?;
    execute(
        &pool,
        "UPDATE track_work_keys SET superseded = true
         WHERE track_id IN (
             SELECT id FROM tracks
             WHERE superseded_by IS NULL AND substr(sc_track_id, 2)::int BETWEEN 5001 AND 5050
         )",
    )
    .await?;
    assert!(drift(&pool).await? > 0);

    let totals = run(&pool).await?;

    assert_eq!(drift(&pool).await?, 0);
    assert_eq!(totals.status, "completed");
    assert_eq!(totals.skipped, 0);
    assert_eq!(totals.changed, 3050);
    assert_eq!(totals.merged, 50);
    assert_eq!(totals.scanned, 3 * 12000);
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn the_backfill_races_live_writers_and_ends_with_no_drift(
    pool: PgPool,
) -> anyhow::Result<()> {
    seed_artists(&pool, 5).await?;
    seed(&pool, 1, 30000).await?;

    let stop = Arc::new(AtomicBool::new(false));
    let writers = [
        tokio::spawn(inserts(pool.clone(), stop.clone())),
        tokio::spawn(repeat(pool.clone(), stop.clone(), MOVE_ARTIST)),
        tokio::spawn(repeat(pool.clone(), stop.clone(), STORE_KEYS)),
    ];
    let totals = run(&pool).await;
    stop.store(true, Ordering::Relaxed);
    for writer in writers {
        assert!(
            writer.await?? > 0,
            "a writer never ran next to the backfill"
        );
    }
    let totals = totals?;

    assert_eq!(totals.status, "completed");
    assert_eq!(totals.skipped, 0);
    assert_eq!(drift(&pool).await?, 0);
    Ok(())
}

async fn inserts(pool: PgPool, stop: Arc<AtomicBool>) -> anyhow::Result<u64> {
    let mut rounds = 0;
    while !stop.load(Ordering::Relaxed) {
        let n = 100_000 + rounds as i32;
        sqlx::query(INSERT).bind(n).execute(&pool).await?;
        rounds += 1;
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
    Ok(rounds)
}

async fn repeat(pool: PgPool, stop: Arc<AtomicBool>, sql: &'static str) -> anyhow::Result<u64> {
    let mut rounds = 0;
    while !stop.load(Ordering::Relaxed) {
        sqlx::query(sql).execute(&pool).await?;
        rounds += 1;
    }
    Ok(rounds)
}

async fn run(pool: &PgPool) -> anyhow::Result<Totals> {
    let run_id = Uuid::now_v7();
    let queue = JobRepository::new(pool.clone(), "track-work-keys-admin-test".to_owned());
    queue.enqueue_if_absent(&job(run_id)).await?;
    let job = queue
        .claim(
            &[JobKind::AdminTrackWorkKeys],
            ClaimOrder::Priority,
            1,
            Duration::from_secs(600),
        )
        .await?
        .pop()
        .ok_or_else(|| anyhow::anyhow!("track work keys job was not claimed"))?;
    let handler = handler(pool);
    for _ in 0..500 {
        handler
            .backfill_track_work_keys(&job, AdminMaintenancePayload { run_id })
            .await?;
        let totals: Totals = sqlx::query_as(
            "SELECT status, scanned, changed, merged, skipped
             FROM admin_maintenance_runs
             WHERE kind = $1",
        )
        .bind(TRACK_WORK_KEYS)
        .fetch_one(pool)
        .await?;
        if totals.status == "completed" {
            return Ok(totals);
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    anyhow::bail!("the track work keys run did not complete")
}

fn job(run_id: Uuid) -> NewJob {
    NewJob {
        id: Uuid::now_v7(),
        kind: JobKind::AdminTrackWorkKeys,
        dedup_key: Some(TRACK_WORK_KEYS.to_owned()),
        payload: serde_json::json!({
            "version": "1",
            "payload": { "run_id": run_id }
        }),
        priority: 20,
        max_attempts: 8,
        available_at: chrono::Utc::now() - chrono::Duration::minutes(1),
    }
}

async fn seed(pool: &PgPool, from: i32, to: i32) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO tracks (
             sc_track_id, urn, title, title_normalized, duration_ms, sharing
         )
         SELECT 's' || n, 'soundcloud:tracks:s' || n, 'Track ' || n, 'track ' || n, 180000, 'public'
         FROM generate_series($1::int, $2::int) AS n",
    )
    .bind(from)
    .bind(to)
    .execute(pool)
    .await?;
    Ok(())
}

async fn seed_artists(pool: &PgPool, count: i32) -> anyhow::Result<Vec<Uuid>> {
    Ok(sqlx::query_scalar(
        "INSERT INTO artists (name, normalized_name, source)
         SELECT 'Artist ' || n, 'artist ' || n, 'soundcloud'
         FROM generate_series(1, $1) AS n
         RETURNING id",
    )
    .bind(count)
    .fetch_all(pool)
    .await?)
}

async fn track_id(pool: &PgPool, n: i32) -> anyhow::Result<Uuid> {
    Ok(
        sqlx::query_scalar("SELECT id FROM tracks WHERE sc_track_id = 's' || $1::int")
            .bind(n)
            .fetch_one(pool)
            .await?,
    )
}

async fn set_artist(pool: &PgPool, track: Uuid, artist: Uuid) -> anyhow::Result<()> {
    sqlx::query("UPDATE tracks SET primary_artist_id = $2 WHERE id = $1")
        .bind(track)
        .bind(artist)
        .execute(pool)
        .await?;
    Ok(())
}

type Mirror = (Option<Uuid>, bool, Option<String>, Option<String>);

async fn mirror(pool: &PgPool, id: Uuid) -> anyhow::Result<Mirror> {
    Ok(sqlx::query_as(
        "SELECT primary_artist_id, superseded, work_key, recording_key
         FROM track_work_keys
         WHERE track_id = $1",
    )
    .bind(id)
    .fetch_one(pool)
    .await?)
}

async fn row_version(pool: &PgPool, id: Uuid) -> anyhow::Result<String> {
    Ok(
        sqlx::query_scalar("SELECT xmin::text FROM track_work_keys WHERE track_id = $1")
            .bind(id)
            .fetch_one(pool)
            .await?,
    )
}

async fn rows_of(pool: &PgPool, id: Uuid) -> anyhow::Result<i64> {
    Ok(
        sqlx::query_scalar("SELECT count(*) FROM track_work_keys WHERE track_id = $1")
            .bind(id)
            .fetch_one(pool)
            .await?,
    )
}

async fn execute(pool: &PgPool, sql: &str) -> anyhow::Result<()> {
    sqlx::query(sql).execute(pool).await?;
    Ok(())
}

async fn drift(pool: &PgPool) -> anyhow::Result<i64> {
    Ok(sqlx::query_scalar(DRIFT).fetch_one(pool).await?)
}
