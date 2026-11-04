use std::sync::Arc;
use std::time::Duration;

use backend_contracts::JobKind;
use sqlx::PgPool;

use super::LookupLane;
use crate::handlers::LOOKUP_KINDS;
use crate::queue::{ClaimOrder, JobRepository, WriteGovernor};

const LEASE: Duration = Duration::from_secs(600);

fn lane(base: usize, limit: usize) -> Arc<LookupLane> {
    let governor = WriteGovernor::new(32);
    governor.hold_at(limit);
    LookupLane::new(base, governor)
}

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

#[test]
fn the_cap_is_the_base_scaled_by_the_governor_limit_and_never_zero() {
    assert_eq!(lane(128, 32).cap(), 128);
    assert_eq!(lane(128, 2).cap(), 8);
    assert_eq!(lane(64, 2).cap(), 4);
    assert_eq!(lane(64, 3).cap(), 6);
    assert_eq!(lane(4, 1).cap(), 1);
}

#[test]
fn a_waiting_targeted_queue_cuts_the_sweep_to_a_quarter_and_an_idle_one_leaves_it_all_but_one() {
    let lane = lane(8, 32);
    lane.sweep_wants(30);
    let mut sweep: Vec<_> = std::iter::from_fn(|| lane.sweep_slot()).collect();
    assert_eq!(sweep.len(), 7);

    let targeted = lane.targeted_slots();
    assert_eq!(targeted.len(), 1);
    lane.targeted_waiting(true);
    sweep.truncate(3);
    assert!(lane.sweep_slot().is_none());
    sweep.truncate(2);
    assert!(lane.sweep_slot().is_none());
    assert_eq!(lane.targeted_slots().len(), 5);

    lane.targeted_waiting(false);
    assert_eq!(lane.sweep_slots(), 7);
}

#[sqlx::test(migrations = false)]
async fn old_lookups_and_an_old_sweep_left_in_other_lanes_are_owned_by_the_lookup_lane(
    pool: PgPool,
) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    sqlx::query(
        "INSERT INTO background_jobs (
             id, kind, lane, dedup_key, payload, priority, available_at,
             lease_id, lease_generation, leased_by, lease_expires_at
         )
         SELECT gen_random_uuid(), 'lyrics.lookup', 'core_bulk', n::text, '{}'::jsonb, 10,
                now() - interval '12 days', NULL, NULL, NULL, NULL
         FROM generate_series(1, 3) AS n
         UNION ALL
         SELECT gen_random_uuid(), 'lyrics.lookup', 'core_bulk', '4', '{}'::jsonb, 10,
                now() - interval '12 days', gen_random_uuid(), 1, 'old-binary',
                now() - interval '1 minute'
         UNION ALL
         SELECT gen_random_uuid(), 'lyrics.lookup_sweep', 'maintenance', 'schedule', '{}'::jsonb,
                -10, now(), NULL, NULL, NULL, NULL",
    )
    .execute(&pool)
    .await?;
    let lane = lane(8, 32);
    let queue = JobRepository::new(pool.clone(), "lookup".to_owned());

    let targeted = lane
        .claim_targeted(&queue, lane.targeted_slots(), LEASE)
        .await?;
    let held = lane.held().targeted;
    let released = queue.release_expired(LOOKUP_KINDS, 16).await?;
    let late = lane
        .claim_targeted(&queue, lane.targeted_slots(), LEASE)
        .await?;
    let sweep = queue
        .claim(
            &[JobKind::LyricsLookupSweep],
            ClaimOrder::Kind(JobKind::LyricsLookupSweep),
            1,
            LEASE,
        )
        .await?;

    assert_eq!(targeted.len(), 3);
    assert_eq!(held, 3);
    assert!(!lane.held().targeted_waiting);
    assert_eq!(released, 1);
    assert_eq!(late.len(), 1);
    assert_eq!(sweep.len(), 1);
    let lanes: Vec<String> =
        sqlx::query_scalar("SELECT DISTINCT lane FROM background_jobs ORDER BY lane")
            .fetch_all(&pool)
            .await?;
    assert_eq!(lanes, ["lookup"]);
    drop(targeted);
    drop(late);
    assert_eq!(lane.held().targeted, 0);
    Ok(())
}
