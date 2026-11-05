use std::time::Duration;

use sqlx::PgPool;
use uuid::Uuid;

use super::{AUTH_CLEANUP_BATCH, AUTH_CLEANUP_ROUNDS, MaintenanceHandler};

async fn backlog(pool: &PgPool, expired: i64) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO login_requests (id, state, code_verifier, expires_at)
         SELECT gen_random_uuid(), 'state-' || gen_random_uuid(), 'verifier',
                timezone('UTC', now()) + CASE WHEN n <= $1 THEN interval '-1 minute' ELSE interval '15 minutes' END
         FROM generate_series(1, $1 + 200) AS n",
    )
    .bind(expired)
    .execute(pool)
    .await?;
    sqlx::query(
        "INSERT INTO link_requests (id, claim_token, mode, expires_at)
         SELECT gen_random_uuid(), 'token-' || gen_random_uuid(), 'pull',
                timezone('UTC', now()) + CASE WHEN n <= $1 THEN interval '-1 minute' ELSE interval '5 minutes' END
         FROM generate_series(1, $1 + 200) AS n",
    )
    .bind(expired)
    .execute(pool)
    .await?;
    Ok(())
}

async fn run_and_count(
    maintenance: &MaintenanceHandler,
    pool: &PgPool,
) -> anyhow::Result<(i64, i64)> {
    maintenance.cleanup_login_requests().await?;
    maintenance.cleanup_link_requests().await?;
    Ok(sqlx::query_as(
        "SELECT (SELECT count(*) FROM login_requests), (SELECT count(*) FROM link_requests)",
    )
    .fetch_one(pool)
    .await?)
}

async fn behind_a_held_slot(
    maintenance: &MaintenanceHandler,
    pool: &PgPool,
) -> anyhow::Result<(i64, i64)> {
    sqlx::raw_sql(
        "CREATE TABLE wal_filler (body text);
         INSERT INTO wal_filler SELECT repeat(md5(n::text), 32) FROM generate_series(1, 24000) AS n;",
    )
    .execute(pool)
    .await?;
    run_and_count(maintenance, pool).await
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn auth_cleanup_runs_are_capped_and_wait_for_lagging_replicas(
    pool: PgPool,
) -> anyhow::Result<()> {
    let capped = AUTH_CLEANUP_BATCH * i64::try_from(AUTH_CLEANUP_ROUNDS)?;
    backlog(&pool, capped + 100).await?;
    let mut maintenance = MaintenanceHandler::new(pool.clone());
    maintenance.auth_cleanup_pause = Duration::ZERO;

    let capped_run = run_and_count(&maintenance, &pool).await?;

    let slot = format!("cleanup_lag_{}", Uuid::new_v4().simple());
    sqlx::query("SELECT pg_create_physical_replication_slot($1, true)")
        .bind(&slot)
        .execute(&pool)
        .await?;
    let held = behind_a_held_slot(&maintenance, &pool).await;
    sqlx::query("SELECT pg_drop_replication_slot($1)")
        .bind(&slot)
        .execute(&pool)
        .await?;
    let resumed = run_and_count(&maintenance, &pool).await?;

    assert_eq!(capped_run, (300, 300), "one run stops at its cap");
    assert_eq!(
        held?,
        (300, 300),
        "a lagging slot stops the run before its first batch"
    );
    assert_eq!(resumed, (200, 200), "the run goes on once the slot is gone");
    Ok(())
}
