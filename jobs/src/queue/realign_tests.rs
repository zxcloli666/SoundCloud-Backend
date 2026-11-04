use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use backend_contracts::JobKind;
use sqlx::PgPool;
use tokio_util::sync::CancellationToken;

use super::{LaneRealign, ROW_PAUSE, STEP, pause_after};
use crate::queue::{ClaimOrder, JobRepository};

async fn install_schema(pool: &PgPool) -> anyhow::Result<()> {
    for migration in [
        include_str!("../../../api/migrations/0057_background_jobs.sql"),
        include_str!("../../../api/migrations/0134_background_jobs_maintenance_lane.sql"),
        include_str!("../../../api/migrations/0307_background_jobs_lookup_lane.sql"),
    ] {
        sqlx::raw_sql(migration).execute(pool).await?;
    }
    Ok(())
}

fn paced(pool: &PgPool, step: i64) -> Arc<LaneRealign> {
    Arc::new(LaneRealign {
        pool: pool.clone(),
        step,
        row_pause: Duration::ZERO,
        repeat: Duration::from_millis(50),
        finished: AtomicBool::new(false),
    })
}

async fn finished(realign: &LaneRealign) -> anyhow::Result<()> {
    tokio::time::timeout(Duration::from_secs(10), async {
        while !realign.finished() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await?;
    Ok(())
}

async fn lanes(pool: &PgPool) -> anyhow::Result<Vec<(String, String, i64)>> {
    Ok(sqlx::query_as(
        "SELECT kind, lane, count(*) FROM background_jobs GROUP BY kind, lane ORDER BY kind, lane",
    )
    .fetch_all(pool)
    .await?)
}

#[sqlx::test(migrations = false)]
async fn idle_jobs_left_in_a_stale_lane_move_and_leased_or_unknown_ones_stay(
    pool: PgPool,
) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    sqlx::query(
        "INSERT INTO background_jobs (
             id, kind, lane, dedup_key, payload,
             lease_id, lease_generation, leased_by, lease_expires_at
         ) VALUES
             (gen_random_uuid(), 'collab.train', 'core_bulk', 'schedule', '{}'::jsonb,
              NULL, NULL, NULL, NULL),
             (gen_random_uuid(), 'lyrics.lookup', 'core_bulk', '1', '{}'::jsonb,
              NULL, NULL, NULL, NULL),
             (gen_random_uuid(), 'lyrics.lookup', 'core_bulk', '2', '{}'::jsonb,
              gen_random_uuid(), 1, 'old-binary', now() + interval '1 minute'),
             (gen_random_uuid(), 'external.kind', 'core_bulk', 'schedule', '{}'::jsonb,
              NULL, NULL, NULL, NULL)",
    )
    .execute(&pool)
    .await?;

    let moved = paced(&pool, 1_000).pass().await;

    assert_eq!(moved, 2);
    assert_eq!(
        lanes(&pool).await?,
        [
            ("collab.train".to_owned(), "maintenance".to_owned(), 1),
            ("external.kind".to_owned(), "core_bulk".to_owned(), 1),
            ("lyrics.lookup".to_owned(), "core_bulk".to_owned(), 1),
            ("lyrics.lookup".to_owned(), "lookup".to_owned(), 1),
        ]
    );
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn the_realignment_walks_the_whole_queue_in_small_steps_and_a_second_pass_moves_nothing(
    pool: PgPool,
) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    sqlx::query(
        "INSERT INTO background_jobs (id, kind, lane, dedup_key, payload, priority)
         SELECT gen_random_uuid(),
                CASE WHEN n % 3 = 0 THEN 'indexing.track' ELSE 'lyrics.lookup' END,
                'core_bulk', n::text, '{}'::jsonb, 10
         FROM generate_series(1, 25) AS n",
    )
    .execute(&pool)
    .await?;
    let realign = paced(&pool, 4);
    let moved = realign.pass().await;
    let again = realign.pass().await;

    assert_eq!(moved, 17);
    assert_eq!(again, 0);
    assert_eq!(
        lanes(&pool).await?,
        [
            ("indexing.track".to_owned(), "core_bulk".to_owned(), 8),
            ("lyrics.lookup".to_owned(), "lookup".to_owned(), 17),
        ]
    );
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn the_bulk_lane_never_claims_old_lookups_and_claims_its_own_work_around_them(
    pool: PgPool,
) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    sqlx::query(
        "INSERT INTO background_jobs (id, kind, lane, dedup_key, payload, priority, available_at)
         SELECT gen_random_uuid(), 'lyrics.lookup', 'core_bulk', n::text, '{}'::jsonb, 10,
                now() - interval '12 days'
         FROM generate_series(1, 40) AS n
         UNION ALL
         SELECT gen_random_uuid(), 'indexing.track', 'core_bulk', 'fresh', '{}'::jsonb, 5, now()",
    )
    .execute(&pool)
    .await?;
    let bulk = JobRepository::new(pool.clone(), "core-bulk".to_owned());
    let lease = Duration::from_secs(60);

    let by_priority = bulk
        .claim(
            crate::handlers::CORE_BULK_KINDS,
            ClaimOrder::Priority,
            8,
            lease,
        )
        .await?;
    let lookups_by_kind = JobRepository::new(pool.clone(), "lookup".to_owned())
        .claim(
            &[JobKind::LyricsLookup],
            ClaimOrder::Kind(JobKind::LyricsLookup),
            8,
            lease,
        )
        .await?;

    assert_eq!(
        by_priority.iter().map(|job| job.kind).collect::<Vec<_>>(),
        [JobKind::IndexTrack]
    );
    assert_eq!(lookups_by_kind.len(), 8);
    let moved_by_claim: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM background_jobs WHERE kind = 'lyrics.lookup' AND lane = 'lookup'",
    )
    .fetch_one(&pool)
    .await?;
    assert_eq!(moved_by_claim, 8);
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn the_task_reports_finished_after_its_pass_and_stays_until_shutdown(
    pool: PgPool,
) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    sqlx::query(
        "INSERT INTO background_jobs (id, kind, lane, dedup_key, payload)
         SELECT gen_random_uuid(), 'lyrics.lookup', 'core_bulk', n::text, '{}'::jsonb
         FROM generate_series(1, 5) AS n",
    )
    .execute(&pool)
    .await?;
    let realign = paced(&pool, 2);
    let cancellation = CancellationToken::new();
    assert!(!realign.finished());

    let task = tokio::spawn(realign.clone().run(cancellation.clone()));
    finished(&realign).await?;
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(!task.is_finished());
    cancellation.cancel();
    task.await??;

    assert_eq!(
        lanes(&pool).await?,
        [("lyrics.lookup".to_owned(), "lookup".to_owned(), 5)]
    );
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn jobs_written_to_a_stale_lane_after_the_first_pass_are_moved_by_a_later_one(
    pool: PgPool,
) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    let realign = paced(&pool, 2);
    let cancellation = CancellationToken::new();
    let task = tokio::spawn(realign.clone().run(cancellation.clone()));
    finished(&realign).await?;

    sqlx::query(
        "INSERT INTO background_jobs (id, kind, lane, dedup_key, payload)
         SELECT gen_random_uuid(), 'lyrics.lookup', 'core_bulk', n::text, '{}'::jsonb
         FROM generate_series(1, 3) AS n",
    )
    .execute(&pool)
    .await?;
    let moved = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let stale: i64 =
                sqlx::query_scalar("SELECT count(*) FROM background_jobs WHERE lane <> 'lookup'")
                    .fetch_one(&pool)
                    .await?;
            if stale == 0 {
                return anyhow::Ok(());
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await;
    cancellation.cancel();
    task.await??;
    moved??;

    assert_eq!(
        lanes(&pool).await?,
        [("lyrics.lookup".to_owned(), "lookup".to_owned(), 3)]
    );
    Ok(())
}

#[test]
fn a_full_step_of_moved_rows_is_paced_under_a_megabyte_of_wal_per_second() {
    let worst_wal_per_row = 18 * 1024;
    let step_wal = STEP as f64 * f64::from(worst_wal_per_row);
    let seconds = pause_after(ROW_PAUSE, STEP).as_secs_f64();

    assert!(step_wal / seconds <= 1024.0 * 1024.0);
    assert_eq!(pause_after(ROW_PAUSE, 0), Duration::ZERO);
}
