use std::time::Duration;

use backend_contracts::{AdminMaintenancePayload, JobKind};
use sqlx::PgPool;
use uuid::Uuid;

use super::super::LYRICS_REFETCH;
use super::super::tests::handler;
use crate::queue::{ClaimOrder, JobRepository, LeasedJob, NewJob};

#[derive(Debug, PartialEq, sqlx::FromRow)]
struct Ledger {
    sc_track_id: String,
    kind: String,
    status: String,
    outcome: Option<String>,
    attempts: i32,
    priority: i16,
}

#[derive(Debug, PartialEq, sqlx::FromRow)]
struct Totals {
    status: String,
    phase: String,
    scanned: i64,
    changed: i64,
    merged: i64,
    skipped: i64,
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn the_seed_arms_only_genius_rows_inside_the_broken_window(
    pool: PgPool,
) -> anyhow::Result<()> {
    track(&pool, "a2", 1).await?;
    cache(
        &pool,
        "a1",
        "genius",
        None,
        Some("text"),
        "2026-10-28 15:59:59",
    )
    .await?;
    cache(
        &pool,
        "a2",
        "genius",
        None,
        Some("text"),
        "2026-10-28 16:00:00",
    )
    .await?;
    cache(
        &pool,
        "a3",
        "genius",
        None,
        Some("text"),
        "2026-11-13 11:58:00",
    )
    .await?;
    cache(
        &pool,
        "a4",
        "genius",
        None,
        Some("text"),
        "2026-11-13 11:58:01",
    )
    .await?;
    cache(
        &pool,
        "a5",
        "lrclib",
        None,
        Some("text"),
        "2026-11-01 00:00:00",
    )
    .await?;
    cache(
        &pool,
        "a6",
        "genius",
        Some("lrclib"),
        Some("text"),
        "2026-11-01 00:00:00",
    )
    .await?;
    cache(&pool, "a7", "genius", None, None, "2026-11-01 00:00:00").await?;

    let (job, run_id) = start(&pool).await?;
    seed_all(&pool, &job, run_id).await?;

    let rows = ledger(&pool).await?;
    assert_eq!(
        rows,
        [
            Ledger {
                sc_track_id: "a2".to_owned(),
                kind: "refetch".to_owned(),
                status: "pending".to_owned(),
                outcome: None,
                attempts: 0,
                priority: 1,
            },
            Ledger {
                sc_track_id: "a3".to_owned(),
                kind: "refetch".to_owned(),
                status: "pending".to_owned(),
                outcome: None,
                attempts: 0,
                priority: 0,
            },
        ]
    );
    let created: Vec<String> =
        sqlx::query_scalar("SELECT cache_created_at::text FROM lyrics_repair ORDER BY sc_track_id")
            .fetch_all(&pool)
            .await?;
    assert_eq!(created, ["2026-10-28 16:00:00", "2026-11-13 11:58:00"]);

    let before = snapshot(&pool).await?;
    rewind(&pool).await?;
    seed_all(&pool, &job, run_id).await?;
    assert_eq!(snapshot(&pool).await?, before);
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn the_seed_rearms_only_failed_and_abandoned_rows(pool: PgPool) -> anyhow::Result<()> {
    for id in ["a2", "a3", "a8", "a9", "a10"] {
        cache(
            &pool,
            id,
            "genius",
            None,
            Some("text"),
            "2026-11-01 00:00:00",
        )
        .await?;
    }
    for (id, kind, outcome) in [
        ("a2", "refetch", "failed"),
        ("a3", "refetch", "replaced"),
        ("a8", "refetch", "abandoned"),
        ("a9", "refetch", "different"),
        ("a10", "description", "stripped"),
    ] {
        sqlx::query(
            "INSERT INTO lyrics_repair (sc_track_id, kind, cache_created_at, status, outcome, attempts, settled_at)
             VALUES ($1, $2, '2026-11-01 00:00:00', 'done', $3, 6, now())",
        )
        .bind(id)
        .bind(kind)
        .bind(outcome)
        .execute(&pool)
        .await?;
    }

    let (job, run_id) = start(&pool).await?;
    seed_all(&pool, &job, run_id).await?;

    let rows: Vec<(String, String, Option<String>, i32)> = sqlx::query_as(
        "SELECT sc_track_id, status, outcome, attempts FROM lyrics_repair ORDER BY sc_track_id",
    )
    .fetch_all(&pool)
    .await?;
    let rows: Vec<(&str, &str, Option<&str>, i32)> = rows
        .iter()
        .map(|(id, status, outcome, attempts)| {
            (id.as_str(), status.as_str(), outcome.as_deref(), *attempts)
        })
        .collect();
    assert_eq!(
        rows,
        [
            ("a10", "done", Some("stripped"), 6),
            ("a2", "pending", None, 0),
            ("a3", "done", Some("replaced"), 6),
            ("a8", "pending", None, 0),
            ("a9", "done", Some("different"), 6),
        ]
    );
    let totals = totals(&pool).await?;
    assert_eq!(totals.scanned, 2);
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn the_drain_waits_while_rows_are_pending_and_completes_when_none_are(
    pool: PgPool,
) -> anyhow::Result<()> {
    for id in ["r1", "r2", "r3", "r4", "r5"] {
        cache(
            &pool,
            id,
            "genius",
            None,
            Some("text"),
            "2026-11-01 00:00:00",
        )
        .await?;
    }
    let (job, run_id) = start(&pool).await?;
    seed_all(&pool, &job, run_id).await?;
    assert_eq!(totals(&pool).await?.phase, "drain");

    let job = next_claim(&pool, &job, run_id).await?;
    step(&pool, &job, run_id).await?;
    let waiting = totals(&pool).await?;
    assert_eq!(
        (waiting.status.as_str(), waiting.phase.as_str()),
        ("running", "drain")
    );
    assert_eq!(
        (
            waiting.scanned,
            waiting.changed,
            waiting.merged,
            waiting.skipped
        ),
        (5, 0, 0, 0)
    );
    assert!(requeued_later(&pool).await?);

    for (id, outcome) in [
        ("r1", "replaced"),
        ("r2", "unchanged"),
        ("r3", "no_metadata"),
        ("r4", "abandoned"),
    ] {
        settle(&pool, id, outcome).await?;
    }
    step(&pool, &job, run_id).await?;
    let waiting = totals(&pool).await?;
    assert_eq!(
        (waiting.status.as_str(), waiting.phase.as_str()),
        ("running", "drain")
    );
    assert_eq!(
        (
            waiting.scanned,
            waiting.changed,
            waiting.merged,
            waiting.skipped
        ),
        (5, 1, 2, 1)
    );

    settle(&pool, "r5", "different").await?;
    step(&pool, &job, run_id).await?;
    let done = totals(&pool).await?;
    assert_eq!(
        (done.status.as_str(), done.phase.as_str()),
        ("completed", "done")
    );
    assert_eq!(
        (done.scanned, done.changed, done.merged, done.skipped),
        (5, 1, 3, 1)
    );
    Ok(())
}

async fn start(pool: &PgPool) -> anyhow::Result<(LeasedJob, Uuid)> {
    let run_id = Uuid::now_v7();
    Ok((claim(pool, run_id).await?, run_id))
}

async fn claim(pool: &PgPool, run_id: Uuid) -> anyhow::Result<LeasedJob> {
    let queue = JobRepository::new(pool.clone(), "lyrics-refetch-admin-test".to_owned());
    queue
        .enqueue_if_absent(&NewJob {
            id: Uuid::now_v7(),
            kind: JobKind::AdminLyricsRefetch,
            dedup_key: Some(LYRICS_REFETCH.to_owned()),
            payload: serde_json::json!({
                "version": "1",
                "payload": { "run_id": run_id }
            }),
            priority: 20,
            max_attempts: 8,
            available_at: chrono::Utc::now() - chrono::Duration::minutes(1),
        })
        .await?;
    let job = queue
        .claim(
            &[JobKind::AdminLyricsRefetch],
            ClaimOrder::Priority,
            1,
            Duration::from_secs(600),
        )
        .await?
        .pop()
        .ok_or_else(|| anyhow::anyhow!("lyrics refetch job was not claimed"))?;
    Ok(job)
}

async fn next_claim(pool: &PgPool, job: &LeasedJob, run_id: Uuid) -> anyhow::Result<LeasedJob> {
    let queue = JobRepository::new(pool.clone(), "lyrics-refetch-admin-test".to_owned());
    queue.complete(job).await?;
    sqlx::query("UPDATE background_jobs SET available_at = now() WHERE dedup_key = $1")
        .bind(LYRICS_REFETCH)
        .execute(pool)
        .await?;
    claim(pool, run_id).await
}

async fn step(pool: &PgPool, job: &LeasedJob, run_id: Uuid) -> anyhow::Result<()> {
    handler(pool)
        .refetch_lyrics(job, AdminMaintenancePayload { run_id })
        .await?;
    Ok(())
}

async fn seed_all(pool: &PgPool, job: &LeasedJob, run_id: Uuid) -> anyhow::Result<()> {
    for _ in 0..10 {
        step(pool, job, run_id).await?;
        if totals(pool).await?.phase == "drain" {
            return Ok(());
        }
    }
    anyhow::bail!("the seed did not reach the drain")
}

async fn rewind(pool: &PgPool) -> anyhow::Result<()> {
    sqlx::query(
        "UPDATE admin_maintenance_runs SET phase = 'seed', cursor_text = NULL WHERE kind = $1",
    )
    .bind(LYRICS_REFETCH)
    .execute(pool)
    .await?;
    Ok(())
}

async fn totals(pool: &PgPool) -> anyhow::Result<Totals> {
    Ok(sqlx::query_as(
        "SELECT status, phase, scanned, changed, merged, skipped
         FROM admin_maintenance_runs
         WHERE kind = $1",
    )
    .bind(LYRICS_REFETCH)
    .fetch_one(pool)
    .await?)
}

async fn requeued_later(pool: &PgPool) -> anyhow::Result<bool> {
    Ok(sqlx::query_scalar(
        "SELECT available_at > now() + interval '30 seconds'
         FROM background_jobs
         WHERE dedup_key = $1",
    )
    .bind(LYRICS_REFETCH)
    .fetch_one(pool)
    .await?)
}

async fn settle(pool: &PgPool, id: &str, outcome: &str) -> anyhow::Result<()> {
    sqlx::query(
        "UPDATE lyrics_repair SET status = 'done', outcome = $2, settled_at = now()
         WHERE sc_track_id = $1",
    )
    .bind(id)
    .bind(outcome)
    .execute(pool)
    .await?;
    Ok(())
}

async fn ledger(pool: &PgPool) -> anyhow::Result<Vec<Ledger>> {
    Ok(sqlx::query_as(
        "SELECT sc_track_id, kind, status, outcome, attempts, priority
         FROM lyrics_repair
         ORDER BY sc_track_id",
    )
    .fetch_all(pool)
    .await?)
}

async fn snapshot(pool: &PgPool) -> anyhow::Result<Vec<String>> {
    Ok(
        sqlx::query_scalar("SELECT repair::text FROM lyrics_repair AS repair ORDER BY sc_track_id")
            .fetch_all(pool)
            .await?,
    )
}

async fn track(pool: &PgPool, id: &str, priority: i16) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO tracks (sc_track_id, urn, title, title_normalized, duration_ms, index_priority)
         VALUES ($1, 'soundcloud:tracks:' || $1, 'Track', 'track', 180000, $2)",
    )
    .bind(id)
    .bind(priority)
    .execute(pool)
    .await?;
    Ok(())
}

async fn cache(
    pool: &PgPool,
    id: &str,
    source: &str,
    plain_source: Option<&str>,
    text: Option<&str>,
    created_at: &str,
) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO lyrics_cache (sc_track_id, source, plain_source, plain_text, synced_lrc, created_at)
         VALUES ($1, $2, $3, $4, CASE WHEN $4::text IS NULL THEN '[00:01.00] x' END, $5::text::timestamp)",
    )
    .bind(id)
    .bind(source)
    .bind(plain_source)
    .bind(text)
    .bind(created_at)
    .execute(pool)
    .await?;
    Ok(())
}
