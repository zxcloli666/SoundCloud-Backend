use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use backend_contracts::{AdminMaintenancePayload, JobKind};
use sqlx::PgPool;
use uuid::Uuid;

use super::super::TRACK_SEARCH;
use super::super::tests::handler;
use crate::queue::{ClaimOrder, Completion, JobRepository, NewJob};

const DRIFT: &str = "SELECT (
        SELECT count(*)
        FROM tracks AS track
        LEFT JOIN track_search AS search ON search.id = track.id
        WHERE track.sharing = 'public'
          AND track.deleted_at IS NULL
          AND track.superseded_by IS NULL
          AND (search.id IS NULL
               OR (track.sc_track_id, track.uploader_sc_user_id, track.title, track.title_normalized,
                   track.metadata_artist, track.uploader_username, track.duration_ms,
                   track.play_count_sc, track_search_variant(track.title_normalized))
                  IS DISTINCT FROM
                  (search.sc_track_id, search.uploader_sc_user_id, search.title, search.title_normalized,
                   search.metadata_artist, search.uploader_username, search.duration_ms,
                   search.play_count_sc, search.variant))
    ) + (
        SELECT count(*)
        FROM track_search AS search
        WHERE NOT EXISTS (
            SELECT 1
            FROM tracks AS track
            WHERE track.id = search.id
              AND track.sharing = 'public'
              AND track.deleted_at IS NULL
              AND track.superseded_by IS NULL
        )
    )";

#[derive(Debug, sqlx::FromRow)]
struct Totals {
    status: String,
    scanned: i64,
    changed: i64,
    merged: i64,
    skipped: i64,
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn the_triggers_keep_track_search_equal_to_the_searchable_tracks(
    pool: PgPool,
) -> anyhow::Result<()> {
    seed(&pool, 1, 4).await?;
    assert_eq!(searchable(&pool).await?, ["s1", "s2", "s3", "s4"]);
    assert_eq!(drift(&pool).await?, 0);

    execute(
        &pool,
        "UPDATE tracks
         SET title = 'Song (Remix)', title_normalized = 'song remix', metadata_artist = 'Label', play_count_sc = 7
         WHERE sc_track_id = 's1'",
    )
    .await?;
    assert!(variant(&pool, "s1").await?);
    assert!(!variant(&pool, "s2").await?);
    assert_eq!(drift(&pool).await?, 0);

    let before = xmin(&pool, "s2").await?;
    execute(
        &pool,
        "UPDATE tracks SET index_priority = 1 WHERE sc_track_id = 's2'",
    )
    .await?;
    execute(
        &pool,
        "UPDATE tracks SET title = title, sharing = sharing WHERE sc_track_id = 's2'",
    )
    .await?;
    assert_eq!(xmin(&pool, "s2").await?, before);

    execute(
        &pool,
        "UPDATE tracks SET sharing = 'private' WHERE sc_track_id = 's2'",
    )
    .await?;
    assert_eq!(searchable(&pool).await?, ["s1", "s3", "s4"]);
    execute(
        &pool,
        "UPDATE tracks SET sharing = 'public' WHERE sc_track_id = 's2'",
    )
    .await?;
    assert_eq!(searchable(&pool).await?, ["s1", "s2", "s3", "s4"]);
    assert_eq!(drift(&pool).await?, 0);

    execute(
        &pool,
        "UPDATE tracks SET deleted_at = now() WHERE sc_track_id = 's3'",
    )
    .await?;
    execute(
        &pool,
        "UPDATE tracks
         SET superseded_by = (SELECT id FROM tracks WHERE sc_track_id = 's1')
         WHERE sc_track_id = 's4'",
    )
    .await?;
    assert_eq!(searchable(&pool).await?, ["s1", "s2"]);
    assert_eq!(drift(&pool).await?, 0);

    execute(&pool, "DELETE FROM tracks WHERE sc_track_id = 's1'").await?;
    assert_eq!(searchable(&pool).await?, ["s2", "s4"]);
    assert_eq!(drift(&pool).await?, 0);
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn the_backfill_fills_what_the_triggers_missed_and_the_check_repairs_wrong_rows(
    pool: PgPool,
) -> anyhow::Result<()> {
    execute(&pool, "ALTER TABLE tracks DISABLE TRIGGER track_search_row").await?;
    seed(&pool, 1, 2500).await?;
    execute(
        &pool,
        "INSERT INTO track_search (
             id, sc_track_id, uploader_sc_user_id, title, title_normalized,
             metadata_artist, uploader_username, duration_ms, play_count_sc, variant
         )
         SELECT id, sc_track_id, uploader_sc_user_id, 'wrong', title_normalized,
                metadata_artist, uploader_username, duration_ms, play_count_sc, false
         FROM tracks
         WHERE substr(sc_track_id, 2)::int <= 100",
    )
    .await?;
    execute(
        &pool,
        "ALTER TABLE tracks ENABLE ALWAYS TRIGGER track_search_row",
    )
    .await?;
    assert_eq!(drift(&pool).await?, 2160 + 90 + 10);

    let totals = run(&pool).await?;

    assert_eq!(drift(&pool).await?, 0);
    assert_eq!(count(&pool).await?, 2250);
    assert_eq!(totals.changed, 2160);
    assert_eq!(totals.merged, 100);
    assert_eq!(totals.skipped, 0);
    assert_eq!(totals.scanned, 3 * 2500);
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn the_backfill_races_live_writers_and_ends_with_no_drift(
    pool: PgPool,
) -> anyhow::Result<()> {
    execute(&pool, "ALTER TABLE tracks DISABLE TRIGGER track_search_row").await?;
    seed(&pool, 1, 6000).await?;
    execute(
        &pool,
        "ALTER TABLE tracks ENABLE ALWAYS TRIGGER track_search_row",
    )
    .await?;
    assert_eq!(count(&pool).await?, 0);

    let stop = Arc::new(AtomicBool::new(false));
    let writers = [
        tokio::spawn(renames(pool.clone(), stop.clone())),
        tokio::spawn(visibility(pool.clone(), stop.clone())),
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

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn each_continuation_waits_its_delay_and_the_last_one_ends_the_job_once(
    pool: PgPool,
) -> anyhow::Result<()> {
    seed(&pool, 1, 20).await?;
    let run_id = Uuid::now_v7();
    let queue = JobRepository::new(pool.clone(), "track-search-finish-test".to_owned());
    queue.enqueue_if_absent(&job(run_id)).await?;
    let handler = handler(&pool);
    let kinds = [JobKind::AdminTrackSearch];
    let claim = || queue.claim(&kinds, ClaimOrder::Priority, 1, Duration::from_secs(600));

    let mut completions = Vec::new();
    for _ in 0..10 {
        let job = claim()
            .await?
            .pop()
            .ok_or_else(|| anyhow::anyhow!("the continuation was not claimable"))?;
        handler
            .backfill_track_search(&job, AdminMaintenancePayload { run_id })
            .await?;
        let completion = queue.complete(&job).await?;
        completions.push(completion);
        if completion == Completion::Completed {
            break;
        }
        assert!(
            claim().await?.is_empty(),
            "a continuation ran before its delay"
        );
        sqlx::query("UPDATE background_jobs SET available_at = now() WHERE dedup_key = $1")
            .bind(TRACK_SEARCH)
            .execute(&pool)
            .await?;
    }

    assert_eq!(
        completions,
        [
            Completion::Superseded,
            Completion::Superseded,
            Completion::Superseded,
            Completion::Completed
        ]
    );
    assert!(claim().await?.is_empty());
    let status: String =
        sqlx::query_scalar("SELECT status FROM admin_maintenance_runs WHERE kind = $1")
            .bind(TRACK_SEARCH)
            .fetch_one(&pool)
            .await?;
    assert_eq!(status, "completed");
    assert_eq!(count(&pool).await?, 18);
    assert_eq!(drift(&pool).await?, 0);
    Ok(())
}

async fn renames(pool: PgPool, stop: Arc<AtomicBool>) -> anyhow::Result<u64> {
    let mut rounds = 0_u64;
    while !stop.load(Ordering::Relaxed) {
        let ids: Vec<String> = (0..20)
            .map(|step| format!("s{}", 2 * ((rounds * 20 + step) % 2500 + 1)))
            .collect();
        sqlx::query(
            "UPDATE tracks
             SET title = 'Track ' || sc_track_id || ' ' || $2::bigint,
                 title_normalized = CASE WHEN $2::bigint % 2 = 0 THEN 'track remix' ELSE 'track' END,
                 play_count_sc = $2::bigint
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

async fn visibility(pool: PgPool, stop: Arc<AtomicBool>) -> anyhow::Result<u64> {
    let mut rounds = 0_u64;
    while !stop.load(Ordering::Relaxed) {
        let ids: Vec<String> = (0..10)
            .map(|step| format!("s{}", 4 * ((rounds * 10 + step) % 1250) + 1))
            .collect();
        sqlx::query(
            "UPDATE tracks
             SET sharing = CASE WHEN sharing = 'public' THEN 'private' ELSE 'public' END,
                 deleted_at = CASE WHEN $2::bigint % 3 = 0 THEN now() END
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
            .map(|step| format!("s{}", 4 * ((rounds * 10 + step) % 1000) + 3))
            .collect();
        let mut transaction = pool.begin().await?;
        sqlx::query("UPDATE tracks SET title = 'rolled back' WHERE sc_track_id = ANY($1)")
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
        let number = 4003 + 4 * (rounds % 499) as i32;
        sqlx::query("DELETE FROM tracks WHERE sc_track_id = $1")
            .bind(format!("s{number}"))
            .execute(&pool)
            .await?;
        seed(&pool, number, number).await?;
        sqlx::query(
            "UPDATE tracks
             SET superseded_by = CASE
                 WHEN superseded_by IS NULL THEN (SELECT id FROM tracks WHERE sc_track_id = $2)
             END
             WHERE sc_track_id = $1",
        )
        .bind(format!("s{}", 4003 + 4 * ((rounds * 7 + 250) % 499)))
        .bind(format!("s{number}"))
        .execute(&pool)
        .await?;
        rounds += 1;
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
    Ok(rounds)
}

async fn run(pool: &PgPool) -> anyhow::Result<Totals> {
    let run_id = Uuid::now_v7();
    let queue = JobRepository::new(pool.clone(), "track-search-admin-test".to_owned());
    queue.enqueue_if_absent(&job(run_id)).await?;
    let job = queue
        .claim(
            &[JobKind::AdminTrackSearch],
            ClaimOrder::Priority,
            1,
            Duration::from_secs(600),
        )
        .await?
        .pop()
        .ok_or_else(|| anyhow::anyhow!("track search job was not claimed"))?;
    let handler = handler(pool);
    for _ in 0..500 {
        handler
            .backfill_track_search(&job, AdminMaintenancePayload { run_id })
            .await?;
        let totals: Totals = sqlx::query_as(
            "SELECT status, scanned, changed, merged, skipped
             FROM admin_maintenance_runs
             WHERE kind = $1",
        )
        .bind(TRACK_SEARCH)
        .fetch_one(pool)
        .await?;
        if totals.status == "completed" {
            return Ok(totals);
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    anyhow::bail!("the track search run did not complete")
}

fn job(run_id: Uuid) -> NewJob {
    NewJob {
        id: Uuid::now_v7(),
        kind: JobKind::AdminTrackSearch,
        dedup_key: Some(TRACK_SEARCH.to_owned()),
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
             sc_track_id, urn, title, title_normalized, uploader_sc_user_id, uploader_username,
             duration_ms, sharing
         )
         SELECT 's' || n, 'soundcloud:tracks:s' || n, 'Track ' || n, 'track ' || n, 'u' || n % 7,
                'artist', 180000, CASE WHEN n % 10 = 0 THEN 'private' ELSE 'public' END
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
    Ok(sqlx::query_scalar("SELECT count(*) FROM track_search")
        .fetch_one(pool)
        .await?)
}

async fn searchable(pool: &PgPool) -> anyhow::Result<Vec<String>> {
    Ok(
        sqlx::query_scalar("SELECT sc_track_id FROM track_search ORDER BY sc_track_id")
            .fetch_all(pool)
            .await?,
    )
}

async fn variant(pool: &PgPool, sc_track_id: &str) -> anyhow::Result<bool> {
    Ok(
        sqlx::query_scalar("SELECT variant FROM track_search WHERE sc_track_id = $1")
            .bind(sc_track_id)
            .fetch_one(pool)
            .await?,
    )
}

async fn xmin(pool: &PgPool, sc_track_id: &str) -> anyhow::Result<String> {
    Ok(
        sqlx::query_scalar("SELECT xmin::text FROM track_search WHERE sc_track_id = $1")
            .bind(sc_track_id)
            .fetch_one(pool)
            .await?,
    )
}
