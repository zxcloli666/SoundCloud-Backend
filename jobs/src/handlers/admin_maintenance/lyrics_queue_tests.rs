use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use backend_contracts::{AdminMaintenancePayload, JobKind};
use sqlx::PgPool;
use uuid::Uuid;

use super::super::LYRICS_LOOKUP_QUEUE;
use super::super::tests::handler;
use crate::queue::{ClaimOrder, JobRepository, LeasedJob, NewJob};

const DRIFT: &str = "SELECT (
        SELECT count(*)
        FROM lyrics_lookup_state AS state
        LEFT JOIN lyrics_lookup_queue AS queue ON queue.track_id = state.track_id
        WHERE queue.track_id IS NULL
           OR (
               state.priority,
               state.next_run_at,
               state.created_at,
               CASE
                   WHEN state.wake_message_id IS NOT NULL AND state.wake_durable_at IS NULL
                       THEN state.updated_at
               END,
               state.attempts,
               state.last_attempt_at,
               state.claim_job_id,
               state.claim_job_generation,
               state.claim_job_lease_id,
               state.claim_state_generation,
               state.claim_expires_at
           ) IS DISTINCT FROM (
               queue.priority,
               queue.next_run_at,
               queue.created_at,
               queue.wake_at,
               queue.attempts,
               queue.last_attempt_at,
               queue.claim_job_id,
               queue.claim_job_generation,
               queue.claim_job_lease_id,
               queue.claim_state_generation,
               queue.claim_expires_at
           )
    ) + (
        SELECT count(*)
        FROM lyrics_lookup_queue AS queue
        WHERE NOT EXISTS (
            SELECT 1 FROM lyrics_lookup_state AS state WHERE state.track_id = queue.track_id
        )
    )";

const CLAIM_TARGET: &str = include_str!("../../../queries/lyrics/lookup_claim_target.sql");
const CLAIM_OLDEST: &str = include_str!("../../../queries/lyrics/lookup_claim_oldest.sql");
const SETTLE_MISS: &str = include_str!("../../../queries/lyrics/lookup_settle_miss.sql");
const SETTLE_RETRY: &str = include_str!("../../../queries/lyrics/lookup_settle_retry.sql");
const SETTLE_FOUND: &str = include_str!("../../../queries/lyrics/lookup_settle_found.sql");
const RELEASE_EXPIRED: &str = include_str!("../../../queries/lyrics/lookup_release_expired.sql");
const MARK_WAKE_DURABLE: &str =
    include_str!("../../../queries/lyrics/lookup_mark_wake_durable.sql");
const LOOKUP_BACKFILL: &str = include_str!("../../../queries/lyrics/lookup_backfill.sql");

#[derive(Debug, sqlx::FromRow)]
struct Queued {
    claim_job_id: Option<Uuid>,
    attempts: i64,
    wake_open: bool,
    priority: i16,
}

#[derive(Debug, sqlx::FromRow)]
struct Totals {
    status: String,
    scanned: i64,
    changed: i64,
    merged: i64,
    skipped: i64,
}

#[sqlx::test(migrations = "../api/migrations")]
async fn claims_settles_and_releases_keep_the_queue_equal_to_the_state(
    pool: PgPool,
) -> anyhow::Result<()> {
    seed(&pool, 1, 3).await?;
    assert_eq!(count(&pool, "lyrics_lookup_queue").await?, 3);
    assert_eq!(drift(&pool).await?, 0);
    let (q1, generation) = state(&pool, "q1").await?;
    assert!(queued(&pool, q1).await?.wake_open);

    mark_wake_durable(&pool, "q1").await?;
    assert!(!queued(&pool, q1).await?.wake_open);
    assert_eq!(drift(&pool).await?, 0);

    let job = leased(&pool, JobKind::LyricsLookup).await?;
    claim(&pool, &job, "q1", 900).await?;
    let claimed = queued(&pool, q1).await?;
    assert_eq!((claimed.claim_job_id, claimed.attempts), (Some(job.id), 1));
    assert_eq!(drift(&pool).await?, 0);

    assert!(settle_miss(&pool, &job, q1, generation).await?);
    assert_eq!(queued(&pool, q1).await?.claim_job_id, None);
    assert_eq!(drift(&pool).await?, 0);

    claim(&pool, &job, "q1", 0).await?;
    let released = sqlx::query(RELEASE_EXPIRED)
        .bind(10_i64)
        .execute(&pool)
        .await?
        .rows_affected();
    assert_eq!(released, 1);
    let released = queued(&pool, q1).await?;
    assert_eq!((released.claim_job_id, released.attempts), (None, 2));
    assert_eq!(drift(&pool).await?, 0);

    claim(&pool, &job, "q1", 900).await?;
    assert!(settle_retry(&pool, &job, q1, generation).await?);
    assert_eq!(queued(&pool, q1).await?.claim_job_id, None);
    assert_eq!(drift(&pool).await?, 0);
    Ok(())
}

#[sqlx::test(migrations = "../api/migrations")]
async fn input_wake_and_priority_changes_keep_the_queue_equal_and_equal_rows_are_not_written(
    pool: PgPool,
) -> anyhow::Result<()> {
    seed(&pool, 1, 2).await?;
    let (q1, generation) = state(&pool, "q1").await?;
    let job = leased(&pool, JobKind::LyricsLookup).await?;

    claim(&pool, &job, "q1", 900).await?;
    sqlx::query("UPDATE tracks SET title = 'Renamed' WHERE sc_track_id = 'q1'")
        .execute(&pool)
        .await?;
    let (_, renamed) = state(&pool, "q1").await?;
    assert_eq!(renamed, generation + 1);
    let reset = queued(&pool, q1).await?;
    assert_eq!(reset.claim_job_id, None);
    assert!(reset.wake_open);
    assert_eq!(drift(&pool).await?, 0);

    sqlx::query("UPDATE tracks SET index_priority = 1 WHERE sc_track_id = 'q1'")
        .execute(&pool)
        .await?;
    assert_eq!(queued(&pool, q1).await?.priority, 1);
    assert_eq!(drift(&pool).await?, 0);

    claim(&pool, &job, "q1", 900).await?;
    assert!(settle_miss(&pool, &job, q1, renamed).await?);
    sqlx::query(
        "SELECT public.scd_cmd_lyrics_wake(jsonb_build_object('sc_track_id', 'q1', 'at', now()))",
    )
    .execute(&pool)
    .await?;
    let woken = queued(&pool, q1).await?;
    assert_eq!(woken.priority, 0);
    assert!(woken.wake_open);
    assert_eq!(drift(&pool).await?, 0);

    let (q2, _) = state(&pool, "q2").await?;
    mark_wake_durable(&pool, "q2").await?;
    let state_before = xmin(&pool, "lyrics_lookup_state", q2).await?;
    let queue_before = xmin(&pool, "lyrics_lookup_queue", q2).await?;
    sqlx::query("UPDATE tracks SET index_priority = 5 WHERE sc_track_id = 'q2'")
        .execute(&pool)
        .await?;
    assert_ne!(xmin(&pool, "lyrics_lookup_state", q2).await?, state_before);
    assert_eq!(xmin(&pool, "lyrics_lookup_queue", q2).await?, queue_before);
    assert_eq!(drift(&pool).await?, 0);

    sqlx::query("DELETE FROM lyrics_lookup_state WHERE sc_track_id = 'q2'")
        .execute(&pool)
        .await?;
    assert_eq!(count(&pool, "lyrics_lookup_queue").await?, 1);
    sqlx::query(LOOKUP_BACKFILL)
        .bind(100_i64)
        .execute(&pool)
        .await?;
    assert!(queued_row(&pool, q2).await?.is_some());
    assert_eq!(drift(&pool).await?, 0);
    Ok(())
}

#[sqlx::test(migrations = "../api/migrations")]
async fn every_state_delete_takes_the_queue_row_with_it(pool: PgPool) -> anyhow::Result<()> {
    seed(&pool, 1, 3).await?;
    let job = leased(&pool, JobKind::LyricsLookup).await?;

    let (q1, g1) = state(&pool, "q1").await?;
    claim(&pool, &job, "q1", 900).await?;
    assert!(settle_found(&pool, &job, q1, g1, "[00:01.00] words", "words", false).await?);
    assert!(queued_row(&pool, q1).await?.is_none());
    assert_eq!(drift(&pool).await?, 0);

    let (q2, g2) = state(&pool, "q2").await?;
    claim(&pool, &job, "q2", 900).await?;
    assert!(settle_found(&pool, &job, q2, g2, "", "words", true).await?);
    assert_eq!(queued(&pool, q2).await?.claim_job_id, None);
    assert_eq!(drift(&pool).await?, 0);
    sqlx::query("UPDATE tracks SET index_priority = 4 WHERE sc_track_id = 'q2'")
        .execute(&pool)
        .await?;
    assert!(queued_row(&pool, q2).await?.is_none());
    assert_eq!(drift(&pool).await?, 0);

    sqlx::query("DELETE FROM tracks WHERE sc_track_id = 'q3'")
        .execute(&pool)
        .await?;
    assert_eq!(count(&pool, "lyrics_lookup_state").await?, 0);
    assert_eq!(count(&pool, "lyrics_lookup_queue").await?, 0);
    Ok(())
}

#[sqlx::test(migrations = "../api/migrations")]
async fn the_backfill_fills_an_empty_queue_and_the_check_repairs_wrong_rows(
    pool: PgPool,
) -> anyhow::Result<()> {
    sqlx::query("ALTER TABLE lyrics_lookup_state DISABLE TRIGGER lyrics_lookup_queue_sync")
        .execute(&pool)
        .await?;
    seed(&pool, 1, 1500).await?;
    sqlx::raw_sql(
        "UPDATE lyrics_lookup_state SET wake_durable_at = now() WHERE sc_track_id LIKE 'q1%';
         UPDATE lyrics_lookup_state SET attempts = 3, last_attempt_at = now() - interval '1 hour'
         WHERE sc_track_id LIKE 'q2%';
         INSERT INTO lyrics_lookup_queue (track_id, priority, next_run_at, created_at, attempts)
         SELECT track_id, priority, next_run_at, created_at, 99
         FROM lyrics_lookup_state
         WHERE sc_track_id LIKE 'q3%';",
    )
    .execute(&pool)
    .await?;
    sqlx::query("ALTER TABLE lyrics_lookup_state ENABLE ALWAYS TRIGGER lyrics_lookup_queue_sync")
        .execute(&pool)
        .await?;
    let wrong: i64 = sqlx::query_scalar("SELECT count(*) FROM lyrics_lookup_queue")
        .fetch_one(&pool)
        .await?;
    assert_eq!(wrong, 111);
    assert_eq!(drift(&pool).await?, 1500);

    let totals = run(&pool).await?;

    assert_eq!(drift(&pool).await?, 0);
    assert_eq!(count(&pool, "lyrics_lookup_queue").await?, 1500);
    assert_eq!(totals.changed, 1500 - wrong);
    assert_eq!(totals.merged, wrong);
    assert_eq!(totals.skipped, 0);
    assert_eq!(totals.scanned, 3 * 1500);
    Ok(())
}

#[sqlx::test(migrations = "../api/migrations")]
async fn the_backfill_races_live_writers_and_ends_with_no_drift(
    pool: PgPool,
) -> anyhow::Result<()> {
    sqlx::raw_sql(
        "ALTER TABLE lyrics_lookup_state SET (fillfactor = 10);
         ALTER TABLE lyrics_lookup_state DISABLE TRIGGER lyrics_lookup_queue_sync;",
    )
    .execute(&pool)
    .await?;
    seed(&pool, 1, 6000).await?;
    sqlx::query("ALTER TABLE lyrics_lookup_state ENABLE ALWAYS TRIGGER lyrics_lookup_queue_sync")
        .execute(&pool)
        .await?;
    assert_eq!(count(&pool, "lyrics_lookup_queue").await?, 0);
    let pages: i64 = sqlx::query_scalar(include_str!(
        "../../../queries/admin_maintenance/lyrics_queue_pages.sql"
    ))
    .fetch_one(&pool)
    .await?;
    assert!(
        pages > 4 * super::PAGES,
        "{pages} pages give too few slices"
    );

    let stop = Arc::new(AtomicBool::new(false));
    let sweep = leased(&pool, JobKind::LyricsLookupSweep).await?;
    let writers = [
        tokio::spawn(claims(pool.clone(), sweep, stop.clone())),
        tokio::spawn(renames(pool.clone(), stop.clone())),
        tokio::spawn(rollbacks(pool.clone(), stop.clone())),
        tokio::spawn(churn(pool.clone(), stop.clone())),
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

async fn claims(pool: PgPool, job: LeasedJob, stop: Arc<AtomicBool>) -> anyhow::Result<u64> {
    let mut rounds = 0;
    while !stop.load(Ordering::Relaxed) {
        let expiring = rounds % 3 == 0;
        let claimed: Vec<(Uuid, String, i64)> = sqlx::query_as(CLAIM_OLDEST)
            .bind(job.id)
            .bind(job.generation)
            .bind(job.lease_id)
            .bind(25_i64)
            .bind(if expiring { 0_i64 } else { 900_i64 })
            .fetch_all(&pool)
            .await?;
        if expiring {
            sqlx::query(RELEASE_EXPIRED)
                .bind(50_i64)
                .execute(&pool)
                .await?;
        } else {
            for (index, (track_id, _, generation)) in claimed.into_iter().enumerate() {
                if index % 2 == 0 {
                    settle_miss(&pool, &job, track_id, generation).await?;
                } else {
                    settle_retry(&pool, &job, track_id, generation).await?;
                }
            }
        }
        rounds += 1;
    }
    Ok(rounds)
}

async fn renames(pool: PgPool, stop: Arc<AtomicBool>) -> anyhow::Result<u64> {
    let mut rounds = 0_u64;
    while !stop.load(Ordering::Relaxed) {
        let ids: Vec<String> = (0..20)
            .map(|step| format!("q{}", 2 * ((rounds * 20 + step) % 1000 + 1)))
            .collect();
        sqlx::query(
            "UPDATE tracks
             SET title = 'Track ' || sc_track_id || ' ' || $2::bigint,
                 index_priority = ($2::bigint % 6)::smallint
             WHERE sc_track_id = ANY($1)",
        )
        .bind(&ids)
        .bind(rounds as i64)
        .execute(&pool)
        .await?;
        rounds += 1;
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
    Ok(rounds)
}

async fn rollbacks(pool: PgPool, stop: Arc<AtomicBool>) -> anyhow::Result<u64> {
    let mut rounds = 0_u64;
    while !stop.load(Ordering::Relaxed) {
        let ids: Vec<String> = (0..10)
            .map(|step| format!("q{}", 2 * ((rounds * 10 + step) % 1000) + 1))
            .collect();
        let mut transaction = pool.begin().await?;
        sqlx::query(
            "UPDATE lyrics_lookup_state
             SET attempts = attempts + 1, updated_at = now()
             WHERE sc_track_id = ANY($1)",
        )
        .bind(&ids)
        .execute(&mut *transaction)
        .await?;
        tokio::time::sleep(Duration::from_millis(15)).await;
        transaction.rollback().await?;
        rounds += 1;
    }
    Ok(rounds)
}

async fn churn(pool: PgPool, stop: Arc<AtomicBool>) -> anyhow::Result<u64> {
    let mut rounds = 0_u64;
    while !stop.load(Ordering::Relaxed) {
        let number = 2001 + (rounds % 1000) as i32;
        sqlx::query("DELETE FROM tracks WHERE sc_track_id = $1")
            .bind(format!("q{number}"))
            .execute(&pool)
            .await?;
        seed(&pool, number, number).await?;
        sqlx::query(
            "SELECT public.scd_cmd_lyrics_wake(jsonb_build_object('sc_track_id', $1::text, 'at', now()))",
        )
        .bind(format!("q{}", 2001 + (rounds * 7 + 500) % 1000))
        .execute(&pool)
        .await?;
        rounds += 1;
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
    Ok(rounds)
}

async fn run(pool: &PgPool) -> anyhow::Result<Totals> {
    let run_id = Uuid::now_v7();
    let queue = JobRepository::new(pool.clone(), "lyrics-queue-admin-test".to_owned());
    queue
        .enqueue_if_absent(&NewJob {
            id: Uuid::now_v7(),
            kind: JobKind::AdminLyricsLookupQueue,
            dedup_key: Some(LYRICS_LOOKUP_QUEUE.to_owned()),
            payload: serde_json::json!({
                "version": "1",
                "payload": { "run_id": run_id }
            }),
            priority: 20,
            max_attempts: 8,
            available_at: chrono::Utc::now(),
        })
        .await?;
    let job = queue
        .claim(
            &[JobKind::AdminLyricsLookupQueue],
            ClaimOrder::Priority,
            1,
            Duration::from_secs(600),
        )
        .await?
        .pop()
        .ok_or_else(|| anyhow::anyhow!("lyrics queue job was not claimed"))?;
    let handler = handler(pool);
    for _ in 0..500 {
        handler
            .backfill_lyrics_lookup_queue(&job, AdminMaintenancePayload { run_id })
            .await?;
        let totals: Totals = sqlx::query_as(
            "SELECT status, scanned, changed, merged, skipped
             FROM admin_maintenance_runs
             WHERE kind = $1",
        )
        .bind(LYRICS_LOOKUP_QUEUE)
        .fetch_one(pool)
        .await?;
        if totals.status == "completed" {
            return Ok(totals);
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    anyhow::bail!("the lyrics queue run did not complete")
}

async fn seed(pool: &PgPool, from: i32, to: i32) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO tracks (
             sc_track_id, urn, title, title_normalized, uploader_username, duration_ms, index_priority
         )
         SELECT 'q' || n, 'soundcloud:tracks:q' || n, 'Track ' || n, 'track ' || n, 'artist', 180000, 5
         FROM generate_series($1::int, $2::int) AS n",
    )
    .bind(from)
    .bind(to)
    .execute(pool)
    .await?;
    Ok(())
}

async fn drift(pool: &PgPool) -> anyhow::Result<i64> {
    Ok(sqlx::query_scalar(DRIFT).fetch_one(pool).await?)
}

async fn count(pool: &PgPool, table: &str) -> anyhow::Result<i64> {
    Ok(sqlx::query_scalar(&format!("SELECT count(*) FROM {table}"))
        .fetch_one(pool)
        .await?)
}

async fn state(pool: &PgPool, sc_track_id: &str) -> anyhow::Result<(Uuid, i64)> {
    Ok(sqlx::query_as(
        "SELECT track_id, generation FROM lyrics_lookup_state WHERE sc_track_id = $1",
    )
    .bind(sc_track_id)
    .fetch_one(pool)
    .await?)
}

async fn queued_row(pool: &PgPool, track_id: Uuid) -> anyhow::Result<Option<Queued>> {
    Ok(sqlx::query_as(
        "SELECT claim_job_id, attempts, wake_at IS NOT NULL AS wake_open, priority
         FROM lyrics_lookup_queue
         WHERE track_id = $1",
    )
    .bind(track_id)
    .fetch_optional(pool)
    .await?)
}

async fn queued(pool: &PgPool, track_id: Uuid) -> anyhow::Result<Queued> {
    queued_row(pool, track_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("no queue row for {track_id}"))
}

async fn xmin(pool: &PgPool, table: &str, track_id: Uuid) -> anyhow::Result<String> {
    Ok(sqlx::query_scalar(&format!(
        "SELECT xmin::text FROM {table} WHERE track_id = $1"
    ))
    .bind(track_id)
    .fetch_one(pool)
    .await?)
}

async fn mark_wake_durable(pool: &PgPool, sc_track_id: &str) -> anyhow::Result<()> {
    let (track_id, generation, wake): (Uuid, i64, Uuid) = sqlx::query_as(
        "SELECT track_id, generation, wake_message_id FROM lyrics_lookup_state WHERE sc_track_id = $1",
    )
    .bind(sc_track_id)
    .fetch_one(pool)
    .await?;
    let marked = sqlx::query(MARK_WAKE_DURABLE)
        .bind(track_id)
        .bind(generation)
        .bind(wake)
        .execute(pool)
        .await?
        .rows_affected();
    anyhow::ensure!(
        marked == 1,
        "the wake of {sc_track_id} was not marked durable"
    );
    Ok(())
}

async fn leased(pool: &PgPool, kind: JobKind) -> anyhow::Result<LeasedJob> {
    let queue = JobRepository::new(pool.clone(), "lyrics-queue-test".to_owned());
    queue
        .enqueue(&NewJob {
            id: Uuid::now_v7(),
            kind,
            dedup_key: None,
            payload: serde_json::json!({}),
            priority: 20,
            max_attempts: 8,
            available_at: chrono::Utc::now(),
        })
        .await?;
    queue
        .claim(&[kind], ClaimOrder::Priority, 1, Duration::from_secs(600))
        .await?
        .pop()
        .ok_or_else(|| anyhow::anyhow!("{kind} job was not claimed"))
}

async fn claim(
    pool: &PgPool,
    job: &LeasedJob,
    sc_track_id: &str,
    seconds: i64,
) -> anyhow::Result<()> {
    let claimed: Vec<(Uuid, String, i64)> = sqlx::query_as(CLAIM_TARGET)
        .bind(job.id)
        .bind(job.generation)
        .bind(job.lease_id)
        .bind(sc_track_id)
        .bind(seconds)
        .fetch_all(pool)
        .await?;
    anyhow::ensure!(claimed.len() == 1, "{sc_track_id} was not claimed");
    Ok(())
}

async fn settle_miss(
    pool: &PgPool,
    job: &LeasedJob,
    track_id: Uuid,
    generation: i64,
) -> anyhow::Result<bool> {
    let settled: Option<Uuid> = sqlx::query_scalar(SETTLE_MISS)
        .bind(job.id)
        .bind(job.generation)
        .bind(job.lease_id)
        .bind(track_id)
        .bind(generation)
        .bind(0_i64)
        .bind("not_found")
        .fetch_optional(pool)
        .await?;
    Ok(settled.is_some())
}

async fn settle_retry(
    pool: &PgPool,
    job: &LeasedJob,
    track_id: Uuid,
    generation: i64,
) -> anyhow::Result<bool> {
    let settled: Option<Uuid> = sqlx::query_scalar(SETTLE_RETRY)
        .bind(job.id)
        .bind(job.generation)
        .bind(job.lease_id)
        .bind(track_id)
        .bind(generation)
        .bind(0_i64)
        .bind("transport")
        .bind("transport")
        .bind(0_i64)
        .fetch_optional(pool)
        .await?;
    Ok(settled.is_some())
}

async fn settle_found(
    pool: &PgPool,
    job: &LeasedJob,
    track_id: Uuid,
    generation: i64,
    synced: &str,
    plain: &str,
    recheck: bool,
) -> anyhow::Result<bool> {
    let settled: Option<(String, Option<String>, Option<String>, Option<String>, bool)> =
        sqlx::query_as(SETTLE_FOUND)
            .bind(job.id)
            .bind(job.generation)
            .bind(job.lease_id)
            .bind(track_id)
            .bind(generation)
            .bind("lrclib")
            .bind(synced)
            .bind(plain)
            .bind(recheck)
            .bind(7_i32)
            .fetch_optional(pool)
            .await?;
    Ok(settled.is_some_and(|row| row.4))
}
