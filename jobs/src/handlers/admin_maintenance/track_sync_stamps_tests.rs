use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use backend_contracts::{AdminMaintenancePayload, JobKind};
use sqlx::PgPool;
use uuid::Uuid;

use super::super::AdminMaintenanceHandler;
use super::super::TRACK_SYNC_STAMPS;
use super::super::tests::{handler, handler_with};
use crate::queue::{ClaimOrder, JobRepository, LeasedJob, NewJob, WriteGovernor};

const DRIFT: &str = "SELECT count(*)
    FROM tracks AS track
    LEFT JOIN track_sync_stamps AS stamp ON stamp.track_id = track.id
    WHERE stamp.track_id IS NULL
       OR stamp.sc_synced_at < track.sc_synced_at";

const SYNC: &str = "WITH synced AS (
        UPDATE tracks
        SET sc_synced_at = clock_timestamp() + $1::interval
        WHERE sc_track_id = 's' || (1 + floor(random() * 30000))::int
        RETURNING id, sc_synced_at
    )
    INSERT INTO track_sync_stamps AS stamp (track_id, sc_synced_at)
    SELECT id, sc_synced_at FROM synced
    ON CONFLICT (track_id) DO UPDATE
    SET sc_synced_at = EXCLUDED.sc_synced_at
    WHERE stamp.sc_synced_at < EXCLUDED.sc_synced_at";

const INSERT: &str = "WITH inserted AS (
        INSERT INTO tracks (
            sc_track_id, urn, title, title_normalized, duration_ms, sharing, sc_synced_at
        )
        VALUES ('s' || $1::int, 'soundcloud:tracks:s' || $1::int, 'Track', 'track', 180000, 'public',
                clock_timestamp())
        RETURNING id, sc_synced_at
    )
    INSERT INTO track_sync_stamps (track_id, sc_synced_at)
    SELECT id, sc_synced_at FROM inserted";

#[derive(Debug, sqlx::FromRow)]
struct Totals {
    status: String,
    scanned: i64,
    changed: i64,
    merged: i64,
    skipped: i64,
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn the_backfill_fills_missing_stamps_and_the_check_repairs_only_older_ones(
    pool: PgPool,
) -> anyhow::Result<()> {
    seed(&pool, 1, 12000).await?;
    execute(
        &pool,
        "INSERT INTO track_sync_stamps (track_id, sc_synced_at)
         SELECT id, sc_synced_at - interval '3 days'
         FROM tracks
         WHERE substr(sc_track_id, 2)::int <= 100",
    )
    .await?;
    execute(
        &pool,
        "INSERT INTO track_sync_stamps (track_id, sc_synced_at)
         SELECT id, sc_synced_at + interval '1 hour'
         FROM tracks
         WHERE substr(sc_track_id, 2)::int BETWEEN 101 AND 150",
    )
    .await?;
    assert_eq!(drift(&pool).await?, 12000 - 50);

    let totals = run(&pool).await?;

    assert_eq!(drift(&pool).await?, 0);
    assert_eq!(count(&pool).await?, 12000);
    assert_eq!(ahead(&pool).await?, 50);
    assert_eq!(totals.changed, 11850);
    assert_eq!(totals.merged, 100);
    assert_eq!(totals.skipped, 0);
    assert_eq!(totals.scanned, 3 * 12000);
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn the_backfill_races_live_writers_and_ends_with_no_drift(
    pool: PgPool,
) -> anyhow::Result<()> {
    seed(&pool, 1, 30000).await?;
    assert_eq!(count(&pool).await?, 0);

    let stop = Arc::new(AtomicBool::new(false));
    let writers = [
        tokio::spawn(syncs(pool.clone(), stop.clone())),
        tokio::spawn(inserts(pool.clone(), stop.clone())),
        tokio::spawn(rollbacks(pool.clone(), stop.clone())),
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

    assert_eq!(totals.skipped, 0);
    assert!(totals.changed > 0);
    assert_eq!(drift(&pool).await?, 0);
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn a_busy_core_holds_the_backfill_until_it_is_calm(pool: PgPool) -> anyhow::Result<()> {
    seed(&pool, 1, 500).await?;
    let governor = WriteGovernor::new(4);
    let handler = handler_with(&pool, governor.clone());
    let run_id = Uuid::now_v7();
    let job = claimed(&pool, run_id).await?;
    for _ in 0..5 {
        handler
            .backfill_track_sync_stamps(&job, AdminMaintenancePayload { run_id })
            .await?;
    }
    assert_eq!(count(&pool).await?, 0);

    governor.hold_calm(true);
    let totals = finish(&pool, &handler, &job, run_id).await?;

    assert_eq!(count(&pool).await?, 500);
    assert_eq!(drift(&pool).await?, 0);
    assert_eq!(totals.skipped, 0);
    Ok(())
}

async fn syncs(pool: PgPool, stop: Arc<AtomicBool>) -> anyhow::Result<u64> {
    let mut rounds = 0;
    while !stop.load(Ordering::Relaxed) {
        sqlx::query(SYNC).bind("0 seconds").execute(&pool).await?;
        rounds += 1;
    }
    Ok(rounds)
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

async fn rollbacks(pool: PgPool, stop: Arc<AtomicBool>) -> anyhow::Result<u64> {
    let mut rounds = 0;
    while !stop.load(Ordering::Relaxed) {
        let mut transaction = pool.begin().await?;
        sqlx::query(SYNC)
            .bind("1 day")
            .execute(&mut *transaction)
            .await?;
        transaction.rollback().await?;
        rounds += 1;
    }
    Ok(rounds)
}

async fn run(pool: &PgPool) -> anyhow::Result<Totals> {
    let run_id = Uuid::now_v7();
    let job = claimed(pool, run_id).await?;
    finish(pool, &handler(pool), &job, run_id).await
}

async fn claimed(pool: &PgPool, run_id: Uuid) -> anyhow::Result<LeasedJob> {
    let queue = JobRepository::new(pool.clone(), "track-sync-stamps-admin-test".to_owned());
    queue.enqueue_if_absent(&job(run_id)).await?;
    queue
        .claim(
            &[JobKind::AdminTrackSyncStamps],
            ClaimOrder::Priority,
            1,
            Duration::from_secs(600),
        )
        .await?
        .pop()
        .ok_or_else(|| anyhow::anyhow!("track sync stamps job was not claimed"))
}

async fn finish(
    pool: &PgPool,
    handler: &AdminMaintenanceHandler,
    job: &LeasedJob,
    run_id: Uuid,
) -> anyhow::Result<Totals> {
    for _ in 0..500 {
        handler
            .backfill_track_sync_stamps(job, AdminMaintenancePayload { run_id })
            .await?;
        let totals: Totals = sqlx::query_as(
            "SELECT status, scanned, changed, merged, skipped
             FROM admin_maintenance_runs
             WHERE kind = $1",
        )
        .bind(TRACK_SYNC_STAMPS)
        .fetch_one(pool)
        .await?;
        if totals.status == "completed" {
            return Ok(totals);
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    anyhow::bail!("the track sync stamps run did not complete")
}

fn job(run_id: Uuid) -> NewJob {
    NewJob {
        id: Uuid::now_v7(),
        kind: JobKind::AdminTrackSyncStamps,
        dedup_key: Some(TRACK_SYNC_STAMPS.to_owned()),
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
             sc_track_id, urn, title, title_normalized, duration_ms, sharing, sc_synced_at
         )
         SELECT 's' || n, 'soundcloud:tracks:s' || n, 'Track ' || n, 'track ' || n, 180000, 'public',
                now() - n * interval '1 minute'
         FROM generate_series($1::int, $2::int) AS n",
    )
    .bind(from)
    .bind(to)
    .execute(pool)
    .await?;
    Ok(())
}

async fn execute(pool: &PgPool, sql: &str) -> anyhow::Result<()> {
    sqlx::query(sql).execute(pool).await?;
    Ok(())
}

async fn drift(pool: &PgPool) -> anyhow::Result<i64> {
    Ok(sqlx::query_scalar(DRIFT).fetch_one(pool).await?)
}

async fn count(pool: &PgPool) -> anyhow::Result<i64> {
    Ok(sqlx::query_scalar("SELECT count(*) FROM track_sync_stamps")
        .fetch_one(pool)
        .await?)
}

async fn ahead(pool: &PgPool) -> anyhow::Result<i64> {
    Ok(sqlx::query_scalar(
        "SELECT count(*)
         FROM track_sync_stamps AS stamp
         JOIN tracks AS track ON track.id = stamp.track_id
         WHERE stamp.sc_synced_at > track.sc_synced_at",
    )
    .fetch_one(pool)
    .await?)
}
