use std::time::Duration;

use backend_contracts::JobKind;
use chrono::{Duration as ChronoDuration, Utc};
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

use super::*;
use crate::queue::model::{ClaimOrder, NewJob};

async fn install_schema(pool: &PgPool) -> anyhow::Result<()> {
    sqlx::raw_sql(include_str!(
        "../../../../api/migrations/0057_background_jobs.sql"
    ))
    .execute(pool)
    .await?;
    Ok(())
}

fn new_job(
    kind: JobKind,
    dedup_key: Option<&str>,
    priority: i16,
    max_attempts: i16,
    available_at: chrono::DateTime<Utc>,
) -> NewJob {
    NewJob {
        id: Uuid::now_v7(),
        kind,
        dedup_key: dedup_key.map(str::to_owned),
        payload: json!({}),
        priority,
        max_attempts,
        available_at,
    }
}

#[test]
fn errors_are_truncated_on_character_boundaries() {
    assert_eq!(truncate("абв", 2), "аб");
}

#[sqlx::test(migrations = false)]
async fn postponing_preserves_retry_budget_and_prevents_early_claim(
    pool: PgPool,
) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    let repository = JobRepository::new(pool.clone(), "postpone-test".into());
    let submitted = new_job(
        JobKind::CatalogRefresh,
        Some("profile:42:42"),
        5,
        1,
        Utc::now(),
    );
    repository.enqueue(&submitted).await?;
    let jobs = repository
        .claim(
            &[JobKind::CatalogRefresh],
            ClaimOrder::Priority,
            1,
            Duration::from_secs(60),
        )
        .await?;
    let [job] = jobs.as_slice() else {
        anyhow::bail!("expected one job")
    };
    assert_eq!(job.attempts, 1);
    assert_eq!(
        repository
            .postpone(job, "refresh cooldown", Duration::from_secs(600))
            .await?,
        Completion::Completed
    );
    assert!(
        repository
            .claim(
                &[JobKind::CatalogRefresh],
                ClaimOrder::Priority,
                1,
                Duration::from_secs(60)
            )
            .await?
            .is_empty()
    );
    let state: (i32, bool, bool) = sqlx::query_as("SELECT attempts, lease_id IS NULL, available_at >= now() + interval '9 minutes' FROM background_jobs WHERE id = $1")
        .bind(job.id).fetch_one(&pool).await?;
    assert_eq!(state, (0, true, true));
    assert_eq!(
        repository
            .postpone(job, "duplicate", Duration::from_secs(1))
            .await?,
        Completion::LostLease
    );
    sqlx::query("UPDATE background_jobs SET available_at = now() - interval '1 second'")
        .execute(&pool)
        .await?;
    let reclaimed = repository
        .claim(
            &[JobKind::CatalogRefresh],
            ClaimOrder::Priority,
            1,
            Duration::from_secs(60),
        )
        .await?;
    let [reclaimed] = reclaimed.as_slice() else {
        anyhow::bail!("postponed job was lost")
    };
    assert_eq!(reclaimed.attempts, 1);
    assert_ne!(reclaimed.lease_id, job.lease_id);
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn postponing_cannot_overwrite_an_expired_lease_or_newer_generation(
    pool: PgPool,
) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    let repository = JobRepository::new(pool.clone(), "postpone-fence-test".into());
    let submitted = new_job(
        JobKind::CatalogRefresh,
        Some("profile:42:42"),
        5,
        3,
        Utc::now(),
    );
    repository.enqueue(&submitted).await?;
    let jobs = repository
        .claim(
            &[JobKind::CatalogRefresh],
            ClaimOrder::Priority,
            1,
            Duration::from_secs(60),
        )
        .await?;
    let [job] = jobs.as_slice() else {
        anyhow::bail!("expected one job")
    };
    sqlx::query("UPDATE background_jobs SET lease_expires_at = now() - interval '1 second'")
        .execute(&pool)
        .await?;
    assert_eq!(
        repository
            .postpone(job, "too late", Duration::from_secs(600))
            .await?,
        Completion::LostLease
    );
    repository
        .enqueue(&NewJob {
            id: Uuid::now_v7(),
            ..submitted
        })
        .await?;
    assert_eq!(
        repository
            .postpone(job, "old generation", Duration::from_secs(600))
            .await?,
        Completion::Superseded
    );
    let state: (i32, bool) =
        sqlx::query_as("SELECT attempts, available_at <= now() FROM background_jobs WHERE id = $1")
            .bind(job.id)
            .fetch_one(&pool)
            .await?;
    assert_eq!(state, (0, true));
    Ok(())
}

#[test]
fn invalid_deduplication_keys_are_rejected_before_querying() {
    assert!(matches!(
        validate_dedup_key(Some(&"x".repeat(MAX_DEDUP_KEY_LENGTH + 1))),
        Err(QueueError::InvalidDedupKey)
    ));
}

#[sqlx::test(migrations = false)]
async fn claim_filters_kinds_and_assigns_per_job_leases(pool: PgPool) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    let repository = JobRepository::new(pool.clone(), "jobs-a".to_owned());
    let now = Utc::now();
    let first = new_job(JobKind::DiscoverAggregates, None, 1, 3, now);
    let second = new_job(JobKind::DiscoverAggregates, None, 1, 3, now);
    let unsupported = new_job(JobKind::CollabTrain, None, 10, 3, now);

    repository.enqueue(&first).await?;
    repository.enqueue(&second).await?;
    repository.enqueue(&unsupported).await?;

    let claimed = repository
        .claim(
            &[JobKind::DiscoverAggregates],
            ClaimOrder::Priority,
            8,
            Duration::from_secs(30),
        )
        .await?;
    let unsupported_state =
        sqlx::query_file!("queries/queue/test_job_lease_state.sql", unsupported.id)
            .fetch_one(&pool)
            .await?;
    let [first_claimed, second_claimed] = claimed.as_slice() else {
        anyhow::bail!("expected two claimed jobs, got {}", claimed.len());
    };

    assert_ne!(first_claimed.lease_id, second_claimed.lease_id);
    assert_eq!(unsupported_state.attempts, 0);
    assert_eq!(unsupported_state.lease_id, None);
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn claim_rejects_kinds_from_different_lanes(pool: PgPool) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    let repository = JobRepository::new(pool, "jobs-a".to_owned());

    let result = repository
        .claim(
            &[JobKind::DiscoverAggregates, JobKind::RecordHardNegative],
            ClaimOrder::Priority,
            8,
            Duration::from_secs(30),
        )
        .await;

    assert!(matches!(result, Err(QueueError::MixedLanes)));
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn enqueue_is_idempotent_by_command_id_and_coalesces_by_key(
    pool: PgPool,
) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    let repository = JobRepository::new(pool.clone(), "jobs-a".to_owned());
    let mut first = new_job(
        JobKind::DiscoverAggregates,
        Some("summary"),
        1,
        3,
        Utc::now(),
    );
    first.payload = json!({ "value": 1 });
    let mut replay = first.clone();
    replay.payload = json!({ "value": 2 });
    let mut next = new_job(
        JobKind::DiscoverAggregates,
        Some("summary"),
        1,
        3,
        Utc::now(),
    );
    next.payload = json!({ "value": 3 });

    repository.enqueue(&first).await?;
    repository.enqueue(&replay).await?;
    repository.enqueue(&next).await?;
    repository.enqueue(&next).await?;

    let state = sqlx::query_file!("queries/queue/test_coalesced_job.sql")
        .fetch_one(&pool)
        .await?;
    let accepted = sqlx::query_file_scalar!("queries/queue/test_count_enqueues.sql")
        .fetch_one(&pool)
        .await?;
    let mut claimed = repository
        .claim(
            &[JobKind::DiscoverAggregates],
            ClaimOrder::Priority,
            1,
            Duration::from_secs(30),
        )
        .await?;
    let claimed = claimed
        .pop()
        .ok_or_else(|| anyhow::anyhow!("expected one claimed job"))?;
    repository.complete(&claimed).await?;
    repository.enqueue(&next).await?;
    let live = sqlx::query_file_scalar!("queries/queue/test_count_jobs.sql")
        .fetch_one(&pool)
        .await?;

    assert_eq!(
        (state.generation, state.payload),
        (2, json!({ "value": 3 }))
    );
    assert_eq!(accepted, 2);
    assert_eq!(live, 0);
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn transactional_enqueue_rolls_back_with_its_caller(pool: PgPool) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    let repository = JobRepository::new(pool.clone(), "jobs-a".to_owned());
    let job = new_job(JobKind::IndexTrack, Some("42"), 0, 8, Utc::now());
    let mut transaction = pool.begin().await?;

    repository.enqueue_in(&mut transaction, &job).await?;
    transaction.rollback().await?;

    let jobs: i64 = sqlx::query_scalar("SELECT count(*) FROM background_jobs")
        .fetch_one(&pool)
        .await?;
    let enqueues: i64 = sqlx::query_scalar("SELECT count(*) FROM background_job_enqueues")
        .fetch_one(&pool)
        .await?;
    assert_eq!((jobs, enqueues), (0, 0));
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn enqueue_if_absent_preserves_an_existing_job(pool: PgPool) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    let repository = JobRepository::new(pool.clone(), "jobs-a".to_owned());
    let mut existing = new_job(
        JobKind::LyricsEmbed,
        Some("42"),
        3,
        5,
        Utc::now() + ChronoDuration::minutes(5),
    );
    existing.payload = json!({ "value": "existing" });
    repository.enqueue(&existing).await?;

    let mut replacement = new_job(JobKind::LyricsEmbed, Some("42"), 10, 8, Utc::now());
    replacement.payload = json!({ "value": "replacement" });
    let mut transaction = pool.begin().await?;
    repository
        .enqueue_in_if_absent(&mut transaction, &replacement)
        .await?;
    transaction.commit().await?;

    let state = sqlx::query_as::<_, (Uuid, serde_json::Value, i16, i64, i32, i16)>(
        "SELECT id, payload, priority, generation, attempts, max_attempts
         FROM background_jobs
         WHERE kind = 'lyrics.embed' AND dedup_key = '42'",
    )
    .fetch_one(&pool)
    .await?;
    assert_eq!(
        state,
        (existing.id, json!({ "value": "existing" }), 3, 1, 0, 5,)
    );
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn newer_generation_cancels_current_lease_before_release(pool: PgPool) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    let repository = JobRepository::new(pool.clone(), "jobs-a".to_owned());
    let first = new_job(
        JobKind::DiscoverAggregates,
        Some("summary"),
        1,
        3,
        Utc::now(),
    );
    repository.enqueue(&first).await?;
    let mut leased = repository
        .claim(
            &[JobKind::DiscoverAggregates],
            ClaimOrder::Priority,
            1,
            Duration::from_secs(30),
        )
        .await?;
    let leased = leased
        .pop()
        .ok_or_else(|| anyhow::anyhow!("expected one claimed job"))?;
    let next = new_job(
        JobKind::DiscoverAggregates,
        Some("summary"),
        1,
        3,
        Utc::now(),
    );

    repository.enqueue(&next).await?;
    let heartbeat = repository
        .heartbeat(&leased, Duration::from_secs(30))
        .await?;
    let completion = repository.complete(&leased).await?;
    let mut replacement = repository
        .claim(
            &[JobKind::DiscoverAggregates],
            ClaimOrder::Priority,
            1,
            Duration::from_secs(30),
        )
        .await?;
    let replacement = replacement
        .pop()
        .ok_or_else(|| anyhow::anyhow!("expected one replacement job"))?;

    assert!(!heartbeat);
    assert_eq!(completion, Completion::Superseded);
    assert_eq!(replacement.generation, 2);
    assert_ne!(replacement.lease_id, leased.lease_id);
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn existing_failure_is_updated_without_losing_live_job_transition(
    pool: PgPool,
) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    let repository = JobRepository::new(pool.clone(), "jobs-a".to_owned());
    let job = new_job(
        JobKind::DiscoverAggregates,
        Some("summary"),
        1,
        1,
        Utc::now(),
    );
    repository.enqueue(&job).await?;
    let mut leased = repository
        .claim(
            &[JobKind::DiscoverAggregates],
            ClaimOrder::Priority,
            1,
            Duration::from_secs(30),
        )
        .await?;
    let leased = leased
        .pop()
        .ok_or_else(|| anyhow::anyhow!("expected one claimed job"))?;
    sqlx::query_file!("queries/queue/test_insert_failure.sql", job.id)
        .execute(&pool)
        .await?;

    let completion = repository.fail(&leased, "current failure", false).await?;
    let live = sqlx::query_file_scalar!("queries/queue/test_count_jobs.sql")
        .fetch_one(&pool)
        .await?;
    let archived_error = sqlx::query_file_scalar!(
        "queries/queue/test_failure_error.sql",
        job.id,
        leased.generation
    )
    .fetch_one(&pool)
    .await?;

    assert_eq!(completion, Completion::Completed);
    assert_eq!(live, 0);
    assert_eq!(archived_error, "current failure");
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn expired_lease_is_released_before_reclaim(pool: PgPool) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    let repository = JobRepository::new(pool.clone(), "jobs-a".to_owned());
    let job = new_job(
        JobKind::DiscoverAggregates,
        Some("summary"),
        1,
        3,
        Utc::now(),
    );
    repository.enqueue(&job).await?;
    let mut stale = repository
        .claim(
            &[JobKind::DiscoverAggregates],
            ClaimOrder::Priority,
            1,
            Duration::from_secs(30),
        )
        .await?;
    let stale = stale
        .pop()
        .ok_or_else(|| anyhow::anyhow!("expected one claimed job"))?;
    sqlx::query_file!("queries/queue/test_expire_job.sql", job.id)
        .execute(&pool)
        .await?;

    let mut current = repository
        .claim(
            &[JobKind::DiscoverAggregates],
            ClaimOrder::Priority,
            1,
            Duration::from_secs(30),
        )
        .await?;
    let current = current
        .pop()
        .ok_or_else(|| anyhow::anyhow!("expected one reclaimed job"))?;
    let stale_completion = repository.complete(&stale).await?;
    let current_heartbeat = repository
        .heartbeat(&current, Duration::from_secs(30))
        .await?;

    assert_eq!(current.attempts, 2);
    assert_ne!(current.lease_id, stale.lease_id);
    assert_eq!(stale_completion, Completion::LostLease);
    assert!(current_heartbeat);
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn exhausted_recovery_is_bounded_and_skips_locked_jobs(pool: PgPool) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    let repository = JobRepository::new(pool.clone(), "jobs-a".to_owned());
    let now = Utc::now();
    for _ in 0..3 {
        repository
            .enqueue(&new_job(JobKind::DiscoverAggregates, None, 1, 1, now))
            .await?;
    }
    repository
        .claim(
            &[JobKind::DiscoverAggregates],
            ClaimOrder::Priority,
            3,
            Duration::from_secs(30),
        )
        .await?;
    sqlx::query_file!("queries/queue/test_expire_all_jobs.sql")
        .execute(&pool)
        .await?;
    let mut transaction = pool.begin().await?;
    sqlx::query_file_scalar!("queries/queue/test_lock_first_job.sql")
        .fetch_one(&mut *transaction)
        .await?;

    let first_pass = repository.recover_exhausted(3).await?;
    transaction.rollback().await?;
    let second_pass = repository.recover_exhausted(1).await?;
    let archived = sqlx::query_file_scalar!("queries/queue/test_count_failures.sql")
        .fetch_one(&pool)
        .await?;

    assert_eq!(first_pass, 2);
    assert_eq!(second_pass, 1);
    assert_eq!(archived, 3);
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn claim_order_can_reserve_capacity_for_oldest_job(pool: PgPool) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    let repository = JobRepository::new(pool.clone(), "jobs-a".to_owned());
    let mut oldest = new_job(
        JobKind::DiscoverAggregates,
        None,
        0,
        3,
        Utc::now() - ChronoDuration::hours(1),
    );
    oldest.payload = json!({ "order": "oldest" });
    let mut priority = new_job(
        JobKind::DiscoverAggregates,
        None,
        10,
        3,
        Utc::now() - ChronoDuration::minutes(1),
    );
    priority.payload = json!({ "order": "priority" });
    let mut second_oldest = oldest.clone();
    second_oldest.id = Uuid::now_v7();
    second_oldest.available_at += ChronoDuration::seconds(1);
    let mut second_priority = priority.clone();
    second_priority.id = Uuid::now_v7();
    second_priority.available_at += ChronoDuration::seconds(1);

    for job in [&oldest, &priority, &second_oldest, &second_priority] {
        repository.enqueue(job).await?;
    }

    let mut priority_claim = repository
        .claim(
            &[JobKind::DiscoverAggregates],
            ClaimOrder::Priority,
            1,
            Duration::from_secs(30),
        )
        .await?;
    let priority_claim = priority_claim
        .pop()
        .ok_or_else(|| anyhow::anyhow!("expected one priority job"))?;
    let mut oldest_claim = repository
        .claim(
            &[JobKind::DiscoverAggregates],
            ClaimOrder::Oldest,
            1,
            Duration::from_secs(30),
        )
        .await?;
    let oldest_claim = oldest_claim
        .pop()
        .ok_or_else(|| anyhow::anyhow!("expected one oldest job"))?;

    assert_eq!(priority_claim.payload, json!({ "order": "priority" }));
    assert_eq!(oldest_claim.payload, json!({ "order": "oldest" }));
    Ok(())
}
