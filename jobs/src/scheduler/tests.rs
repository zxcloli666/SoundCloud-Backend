use sqlx::PgPool;
use uuid::Uuid;

use super::*;
use crate::queue::{ClaimOrder, Completion, JobRepository};

async fn install_schema(pool: &PgPool) -> anyhow::Result<()> {
    sqlx::raw_sql(include_str!(
        "../../../api/migrations/0057_background_jobs.sql"
    ))
    .execute(pool)
    .await?;
    Ok(())
}

#[test]
fn retry_delay_grows_and_stays_bounded() {
    let seed = 42;

    assert!(retry_delay(2, seed) > retry_delay(1, seed));
    assert!(retry_delay(u32::MAX, seed) <= MAX_RETRY_DELAY);
}

#[test]
fn recommendation_schedules_preserve_previous_cadence() {
    let schedules: Vec<_> = SCHEDULES
        .iter()
        .filter(|schedule| {
            matches!(
                schedule.kind,
                JobKind::RecommendationColike | JobKind::RecommendationWavePriority
            )
        })
        .map(|schedule| {
            (
                schedule.kind,
                schedule.interval_seconds,
                schedule.priority,
                schedule.max_attempts,
            )
        })
        .collect();

    assert_eq!(
        schedules,
        vec![
            (JobKind::RecommendationColike, 6 * 60 * 60, -10, 4),
            (JobKind::RecommendationWavePriority, 60 * 60, -5, 4),
        ]
    );
}

#[test]
fn recommendation_quality_schedules_preserve_previous_cadence() {
    let schedules = SCHEDULES
        .iter()
        .filter(|schedule| {
            matches!(
                schedule.kind,
                JobKind::RecommendationQualityBackfill | JobKind::RecommendationQualityTrain
            )
        })
        .map(|schedule| (schedule.kind, schedule.interval_seconds))
        .collect::<Vec<_>>();

    assert_eq!(
        schedules,
        vec![
            (JobKind::RecommendationQualityBackfill, 10 * 60),
            (JobKind::RecommendationQualityTrain, 6 * 60 * 60),
        ]
    );
}

#[test]
fn discover_interest_preserves_previous_cadence() {
    let schedule = SCHEDULES
        .iter()
        .find(|schedule| schedule.kind == JobKind::DiscoverInterest)
        .expect("discover interest schedule");

    assert_eq!(schedule.interval_seconds, 60 * 60);
    assert_eq!(schedule.priority, -10);
    assert_eq!(schedule.max_attempts, 4);
}

#[test]
fn indexing_reap_preserves_previous_cadence() {
    let schedule = SCHEDULES
        .iter()
        .find(|schedule| schedule.kind == JobKind::IndexingReap)
        .expect("indexing reap schedule");

    assert_eq!(schedule.interval_seconds, 5 * 60);
    assert_eq!(schedule.priority, 5);
    assert_eq!(schedule.max_attempts, 8);
}

#[test]
fn lyrics_reapers_preserve_the_previous_cadence() {
    let schedules = SCHEDULES
        .iter()
        .filter(|schedule| {
            matches!(
                schedule.kind,
                JobKind::LyricsReapEmbeddings | JobKind::LyricsReapTranscriptions
            )
        })
        .map(|schedule| {
            (
                schedule.kind,
                schedule.interval_seconds,
                schedule.priority,
                schedule.max_attempts,
                schedule.initial_delay_seconds,
            )
        })
        .collect::<Vec<_>>();

    assert_eq!(
        schedules,
        vec![
            (JobKind::LyricsReapEmbeddings, 10 * 60, 5, 8, 10 * 60),
            (JobKind::LyricsReapTranscriptions, 10 * 60, 5, 8, 10 * 60,),
        ]
    );
}

#[test]
fn duration_resolver_uses_the_capacity_safe_default_cadence() {
    let schedule = SCHEDULES
        .iter()
        .find(|schedule| schedule.kind == JobKind::ResolveDurations)
        .expect("duration resolver schedule");

    assert_eq!(schedule.interval_seconds, 60);
    assert_eq!(schedule.priority, 5);
    assert_eq!(schedule.max_attempts, 8);
}

#[tokio::test]
async fn recommendation_schedule_overrides_are_owned_by_jobs() {
    let pool = sqlx::postgres::PgPoolOptions::new()
        .connect_lazy("postgres://localhost/scheduler-test")
        .expect("lazy test pool");
    let scheduler = Scheduler::configured(
        pool,
        &JobScheduleConfig {
            collab_train_enabled: false,
            collab_train_seconds: 7_654,
            duration_resolver_seconds: 8_765,
            recommendation_colike_seconds: 12_345,
            recommendation_quality_backfill_seconds: 2_345,
            recommendation_quality_train_seconds: 3_456,
            recommendation_wave_priority_seconds: 6_789,
            recommendation_wave_priority_shards: 16,
            discover_interest_seconds: 4_321,
            discover_interest_shards: 8,
            discover_artist_plays_shards: 8,
            discover_interest_enabled: false,
            enrich_enabled: true,
            catalog_crawl_enabled: false,
            catalog_crawl_seconds: 5_432,
            wanted_resolve_seconds: 9_876,
            lyrics_lookup_seconds: 7_777,
            playlist_reconcile_sweep_seconds: 8_888,
        },
    );

    let collab = scheduler
        .schedules
        .iter()
        .find(|schedule| schedule.kind == JobKind::CollabTrain)
        .expect("collab train schedule");
    assert_eq!(collab.interval_seconds, 7_654);
    assert_eq!(collab.initial_delay_seconds, 7_654);
    assert_eq!(collab.enabled, Some(false));
    assert_eq!(
        scheduler
            .schedules
            .iter()
            .find(|schedule| schedule.kind == JobKind::ResolveDurations)
            .map(|schedule| schedule.interval_seconds),
        Some(8_765)
    );

    assert_eq!(
        scheduler
            .schedules
            .iter()
            .find(|schedule| schedule.kind == JobKind::RecommendationColike)
            .map(|schedule| schedule.interval_seconds),
        Some(12_345)
    );
    assert_eq!(
        scheduler
            .schedules
            .iter()
            .find(|schedule| schedule.kind == JobKind::RecommendationWavePriority)
            .map(|schedule| schedule.interval_seconds),
        Some(6_789)
    );
    assert_eq!(
        scheduler
            .schedules
            .iter()
            .find(|schedule| schedule.kind == JobKind::RecommendationQualityBackfill)
            .map(|schedule| schedule.interval_seconds),
        Some(2_345)
    );
    assert_eq!(
        scheduler
            .schedules
            .iter()
            .find(|schedule| schedule.kind == JobKind::RecommendationQualityTrain)
            .map(|schedule| schedule.interval_seconds),
        Some(3_456)
    );
    let discover = scheduler
        .schedules
        .iter()
        .find(|schedule| schedule.kind == JobKind::DiscoverInterest)
        .expect("discover interest schedule");
    assert_eq!(discover.interval_seconds, 4_321);
    assert_eq!(discover.enabled, Some(false));
}

#[sqlx::test(migrations = false)]
async fn scheduler_recovers_after_database_operation_fails(pool: PgPool) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    let scheduler = Scheduler::new(pool.clone());
    scheduler.register().await?;
    sqlx::query("ALTER TABLE background_schedules RENAME TO unavailable_schedules")
        .execute(&pool)
        .await?;

    let shutdown = CancellationToken::new();
    let task = tokio::spawn(scheduler.run(shutdown.clone(), HealthState::new()));
    tokio::time::sleep(Duration::from_millis(100)).await;
    sqlx::query("ALTER TABLE unavailable_schedules RENAME TO background_schedules")
        .execute(&pool)
        .await?;

    tokio::time::timeout(Duration::from_secs(4), async {
        loop {
            let jobs: i64 = sqlx::query_scalar("SELECT count(*) FROM background_jobs")
                .fetch_one(&pool)
                .await?;
            if jobs > 0 {
                return Ok::<_, sqlx::Error>(());
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await??;

    shutdown.cancel();
    task.await??;
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn schedule_tick_does_not_supersede_running_job(pool: PgPool) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    let scheduler = Scheduler::new(pool.clone());
    scheduler.register().await?;
    scheduler.enqueue_due().await?;

    let repository = JobRepository::new(pool.clone(), "scheduler-test".to_owned());
    let mut claimed = repository
        .claim(
            &[JobKind::AuthCleanupLinkRequests],
            ClaimOrder::Priority,
            1,
            Duration::from_secs(30),
        )
        .await?;
    let job = claimed
        .pop()
        .ok_or_else(|| anyhow::anyhow!("job missing"))?;

    sqlx::query(
        "UPDATE background_schedules SET next_run_at = now()
         WHERE kind = 'auth.cleanup_link_requests'",
    )
    .execute(&pool)
    .await?;
    scheduler.enqueue_due().await?;

    assert_eq!(repository.complete(&job).await?, Completion::Completed);
    sqlx::query(
        "UPDATE background_schedules SET next_run_at = now()
         WHERE kind = 'auth.cleanup_link_requests'",
    )
    .execute(&pool)
    .await?;
    scheduler.enqueue_due().await?;
    let next = repository
        .claim(
            &[JobKind::AuthCleanupLinkRequests],
            ClaimOrder::Priority,
            1,
            Duration::from_secs(30),
        )
        .await?
        .pop()
        .ok_or_else(|| anyhow::anyhow!("next job missing"))?;
    assert_ne!(next.id, job.id);
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn schedule_tick_preserves_pending_retry_state(pool: PgPool) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    let scheduler = Scheduler::new(pool.clone());
    scheduler.register().await?;
    scheduler.enqueue_due().await?;

    let state: (Uuid, i64) = sqlx::query_as(
        "UPDATE background_jobs
         SET attempts = 2,
             available_at = now() + interval '10 minutes',
             last_error = 'temporary failure'
         WHERE kind = 'auth.cleanup_link_requests'
         RETURNING id, extract(epoch FROM available_at)::bigint",
    )
    .fetch_one(&pool)
    .await?;
    sqlx::query(
        "UPDATE background_schedules SET next_run_at = now()
         WHERE kind = 'auth.cleanup_link_requests'",
    )
    .execute(&pool)
    .await?;
    scheduler.enqueue_due().await?;

    let after: (Uuid, i32, i64, Option<String>) = sqlx::query_as(
        "SELECT id, attempts, extract(epoch FROM available_at)::bigint, last_error
         FROM background_jobs
         WHERE kind = 'auth.cleanup_link_requests'",
    )
    .fetch_one(&pool)
    .await?;
    assert_eq!(
        after,
        (state.0, 2, state.1, Some("temporary failure".to_owned()))
    );
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn registration_preserves_manual_and_unowned_schedules(pool: PgPool) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    let scheduler = Scheduler::new(pool.clone());
    scheduler.register().await?;
    sqlx::query(
        "UPDATE background_schedules SET enabled = false
         WHERE kind = 'auth.cleanup_link_requests'",
    )
    .execute(&pool)
    .await?;
    sqlx::query(
        "INSERT INTO background_schedules (
             kind, lane, interval_seconds, next_run_at, priority, max_attempts
         ) VALUES ('external.schedule', 'core_bulk', 60, now(), 0, 3)",
    )
    .execute(&pool)
    .await?;

    scheduler.register().await?;

    let current: bool = sqlx::query_scalar(
        "SELECT enabled FROM background_schedules
         WHERE kind = 'auth.cleanup_link_requests'",
    )
    .fetch_one(&pool)
    .await?;
    let external: bool = sqlx::query_scalar(
        "SELECT enabled FROM background_schedules WHERE kind = 'external.schedule'",
    )
    .fetch_one(&pool)
    .await?;
    assert!(!current);
    assert!(external);
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn configured_schedule_owns_its_enabled_state(pool: PgPool) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    let scheduler = Scheduler::configured(
        pool.clone(),
        &JobScheduleConfig {
            collab_train_enabled: true,
            collab_train_seconds: 6 * 60 * 60,
            duration_resolver_seconds: 2 * 60,
            recommendation_colike_seconds: 6 * 60 * 60,
            recommendation_quality_backfill_seconds: 10 * 60,
            recommendation_quality_train_seconds: 6 * 60 * 60,
            recommendation_wave_priority_seconds: 60 * 60,
            recommendation_wave_priority_shards: 16,
            discover_interest_seconds: 60 * 60,
            discover_interest_shards: 8,
            discover_artist_plays_shards: 8,
            discover_interest_enabled: false,
            enrich_enabled: true,
            catalog_crawl_enabled: true,
            catalog_crawl_seconds: 60,
            wanted_resolve_seconds: 60,
            lyrics_lookup_seconds: 60,
            playlist_reconcile_sweep_seconds: 60,
        },
    );

    scheduler.register().await?;

    let enabled: bool = sqlx::query_scalar(
        "SELECT enabled FROM background_schedules WHERE kind = 'discover.interest'",
    )
    .fetch_one(&pool)
    .await?;
    assert!(!enabled);
    Ok(())
}
