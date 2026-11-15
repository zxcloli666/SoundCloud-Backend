use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use backend_contracts::{AdminMaintenancePayload, JobKind};
use sqlx::PgPool;
use uuid::Uuid;

use super::super::TRACK_ENRICH_QUEUE;
use super::super::tests::handler;
use crate::queue::{ClaimOrder, JobRepository, NewJob};

const DRIFT: &str = "SELECT count(*)
    FROM tracks AS track
    FULL JOIN track_enrich_queue AS queue ON queue.track_id = track.id
    WHERE track.id IS NULL
       OR queue.track_id IS NULL
       OR (track.enrich_state, track.index_priority)
          IS DISTINCT FROM
          (queue.enrich_state, queue.index_priority)";

const CLAIM: &str = "UPDATE tracks
    SET enrich_locked_at = now(),
        enrich_attempts = enrich_attempts + 1
    WHERE sc_track_id = 's' || (1 + floor(random() * 30000))::int";

const FAIL: &str = "UPDATE tracks
    SET enrich_state = 'failed',
        enrich_locked_at = NULL,
        enrich_next_run_at = now() + interval '1 hour',
        enrich_error = 'timeout ' || floor(random() * 1000)::int
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

type Lease = (String, i16, bool, i16, Option<String>);

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn the_triggers_copy_the_lease_of_every_track(pool: PgPool) -> anyhow::Result<()> {
    seed(&pool, 1, 2).await?;
    let first = track_id(&pool, 1).await?;
    let second = track_id(&pool, 2).await?;

    assert_eq!(
        lease(&pool, first).await?,
        ("pending".to_owned(), 5, false, 0, None)
    );
    assert_eq!(drift(&pool).await?, 0);

    execute_for(
        &pool,
        first,
        "UPDATE tracks
         SET enrich_locked_at = now(), enrich_attempts = enrich_attempts + 1
         WHERE id = $1",
    )
    .await?;
    assert_eq!(
        lease(&pool, first).await?,
        ("pending".to_owned(), 5, true, 1, None)
    );

    execute_for(
        &pool,
        first,
        "UPDATE tracks
         SET enrich_state = 'failed', enrich_locked_at = NULL, enrich_error = 'timeout',
             enrich_next_run_at = now() + interval '1 hour', index_priority = 1
         WHERE id = $1",
    )
    .await?;
    assert_eq!(
        lease(&pool, first).await?,
        ("failed".to_owned(), 1, false, 1, Some("timeout".to_owned()))
    );
    assert_eq!(drift(&pool).await?, 0);

    let before = row_version(&pool, first).await?;
    execute_for(
        &pool,
        first,
        "UPDATE tracks SET title = 'Other title' WHERE id = $1",
    )
    .await?;
    assert_eq!(row_version(&pool, first).await?, before);

    execute_for(&pool, first, "DELETE FROM tracks WHERE id = $1").await?;
    assert_eq!(rows_of(&pool, first).await?, 0);
    assert_eq!(rows_of(&pool, second).await?, 1);
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn the_backfill_and_the_check_repair_missing_and_stale_rows(
    pool: PgPool,
) -> anyhow::Result<()> {
    seed(&pool, 1, 12000).await?;
    execute(
        &pool,
        "DELETE FROM track_enrich_queue
         WHERE track_id IN (
             SELECT id FROM tracks WHERE substr(sc_track_id, 2)::int BETWEEN 2951 AND 3050
         )",
    )
    .await?;
    execute(
        &pool,
        "UPDATE track_enrich_queue SET enrich_state = 'failed', index_priority = 1
         WHERE track_id IN (
             SELECT id FROM tracks WHERE substr(sc_track_id, 2)::int BETWEEN 5001 AND 5050
         )",
    )
    .await?;
    assert!(drift(&pool).await? > 0);

    let totals = run(&pool).await?;

    assert_eq!(drift(&pool).await?, 0);
    assert_eq!(totals.status, "completed");
    assert_eq!(totals.skipped, 0);
    assert_eq!(totals.changed, 100);
    assert_eq!(totals.merged, 50);
    assert_eq!(totals.scanned, 3 * 12000);
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn the_backfill_races_live_writers_and_ends_with_no_drift(
    pool: PgPool,
) -> anyhow::Result<()> {
    seed(&pool, 1, 30000).await?;
    execute(&pool, "DELETE FROM track_enrich_queue").await?;

    let stop = Arc::new(AtomicBool::new(false));
    let writers = [
        tokio::spawn(inserts(pool.clone(), stop.clone())),
        tokio::spawn(repeat(pool.clone(), stop.clone(), CLAIM)),
        tokio::spawn(repeat(pool.clone(), stop.clone(), FAIL)),
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
    let queue = JobRepository::new(pool.clone(), "track-enrich-queue-admin-test".to_owned());
    queue.enqueue_if_absent(&job(run_id)).await?;
    let job = queue
        .claim(
            &[JobKind::AdminTrackEnrichQueue],
            ClaimOrder::Priority,
            1,
            Duration::from_secs(600),
        )
        .await?
        .pop()
        .ok_or_else(|| anyhow::anyhow!("track enrich queue job was not claimed"))?;
    let handler = handler(pool);
    for _ in 0..500 {
        handler
            .backfill_track_enrich_queue(&job, AdminMaintenancePayload { run_id })
            .await?;
        let totals: Totals = sqlx::query_as(
            "SELECT status, scanned, changed, merged, skipped
             FROM admin_maintenance_runs
             WHERE kind = $1",
        )
        .bind(TRACK_ENRICH_QUEUE)
        .fetch_one(pool)
        .await?;
        if totals.status == "completed" {
            return Ok(totals);
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    anyhow::bail!("the track enrich queue run did not complete")
}

fn job(run_id: Uuid) -> NewJob {
    NewJob {
        id: Uuid::now_v7(),
        kind: JobKind::AdminTrackEnrichQueue,
        dedup_key: Some(TRACK_ENRICH_QUEUE.to_owned()),
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

async fn track_id(pool: &PgPool, n: i32) -> anyhow::Result<Uuid> {
    Ok(
        sqlx::query_scalar("SELECT id FROM tracks WHERE sc_track_id = 's' || $1::int")
            .bind(n)
            .fetch_one(pool)
            .await?,
    )
}

async fn lease(pool: &PgPool, id: Uuid) -> anyhow::Result<Lease> {
    Ok(sqlx::query_as(
        "SELECT enrich_state::text, index_priority, locked_at IS NOT NULL, attempts, error
         FROM track_enrich_queue
         WHERE track_id = $1",
    )
    .bind(id)
    .fetch_one(pool)
    .await?)
}

async fn row_version(pool: &PgPool, id: Uuid) -> anyhow::Result<String> {
    Ok(
        sqlx::query_scalar("SELECT xmin::text FROM track_enrich_queue WHERE track_id = $1")
            .bind(id)
            .fetch_one(pool)
            .await?,
    )
}

async fn rows_of(pool: &PgPool, id: Uuid) -> anyhow::Result<i64> {
    Ok(
        sqlx::query_scalar("SELECT count(*) FROM track_enrich_queue WHERE track_id = $1")
            .bind(id)
            .fetch_one(pool)
            .await?,
    )
}

async fn execute(pool: &PgPool, sql: &str) -> anyhow::Result<()> {
    sqlx::query(sql).execute(pool).await?;
    Ok(())
}

async fn execute_for(pool: &PgPool, id: Uuid, sql: &str) -> anyhow::Result<()> {
    sqlx::query(sql).bind(id).execute(pool).await?;
    Ok(())
}

async fn drift(pool: &PgPool) -> anyhow::Result<i64> {
    Ok(sqlx::query_scalar(DRIFT).fetch_one(pool).await?)
}
