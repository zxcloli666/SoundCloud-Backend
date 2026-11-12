use std::time::Duration;

use metrics_exporter_prometheus::{PrometheusBuilder, PrometheusHandle};
use sqlx::{Connection, PgPool};
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

async fn fill_wal(pool: &PgPool) -> anyhow::Result<()> {
    sqlx::raw_sql(
        "CREATE TABLE IF NOT EXISTS wal_filler (body text);
         INSERT INTO wal_filler SELECT repeat(md5(n::text), 32) FROM generate_series(1, 24000) AS n;",
    )
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

async fn slot_gone(pool: &PgPool, slot: &str) -> anyhow::Result<()> {
    for _ in 0..100 {
        let present: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM pg_replication_slots WHERE slot_name = $1)",
        )
        .bind(slot)
        .fetch_one(pool)
        .await?;
        if !present {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    anyhow::bail!("slot {slot} still exists after its session closed")
}

fn sample(handle: &PrometheusHandle, name: &str, labels: &[&str]) -> Option<f64> {
    handle.render().lines().find_map(|line| {
        let (series, value) = line.rsplit_once(' ')?;
        let matches = series.starts_with(&format!("{name}{{"))
            && labels.iter().all(|label| series.contains(label));
        matches.then(|| value.parse().ok()).flatten()
    })
}

fn progress(handle: &PrometheusHandle) -> (Option<f64>, Option<f64>) {
    let name = "auth_cleanup_last_progress_timestamp_seconds";
    (
        sample(handle, name, &["table=\"login_requests\""]),
        sample(handle, name, &["table=\"link_requests\""]),
    )
}

fn skipped(handle: &PrometheusHandle, why: &str) -> (Option<f64>, Option<f64>) {
    let name = "auth_cleanup_skipped_total";
    let why = format!("why=\"{why}\"");
    (
        sample(handle, name, &["table=\"login_requests\"", &why]),
        sample(handle, name, &["table=\"link_requests\"", &why]),
    )
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn auth_cleanup_waits_only_for_active_lagging_replicas(pool: PgPool) -> anyhow::Result<()> {
    let recorder = PrometheusBuilder::new().build_recorder();
    let handle = recorder.handle();
    let _recording = metrics::set_default_local_recorder(&recorder);
    let capped = AUTH_CLEANUP_BATCH * i64::try_from(AUTH_CLEANUP_ROUNDS)?;
    let mut maintenance = MaintenanceHandler::new(pool.clone());
    maintenance.auth_cleanup_pause = Duration::ZERO;

    backlog(&pool, capped + 100).await?;
    let capped_run = run_and_count(&maintenance, &pool).await?;
    let after_cap = (progress(&handle), skipped(&handle, "batch_cap"));

    let idle = format!("cleanup_idle_{}", Uuid::new_v4().simple());
    sqlx::query("SELECT pg_create_physical_replication_slot($1, true)")
        .bind(&idle)
        .execute(&pool)
        .await?;
    let past_idle = match fill_wal(&pool).await {
        Ok(()) => run_and_count(&maintenance, &pool).await,
        Err(error) => Err(error),
    };
    sqlx::query("SELECT pg_drop_replication_slot($1)")
        .bind(&idle)
        .execute(&pool)
        .await?;
    let after_idle = progress(&handle);

    backlog(&pool, 100).await?;
    let mut consumer = pool.acquire().await?.detach();
    let live = format!("cleanup_live_{}", Uuid::new_v4().simple());
    sqlx::query("SELECT pg_create_physical_replication_slot($1, true, true)")
        .bind(&live)
        .execute(&mut consumer)
        .await?;
    let behind_live = match fill_wal(&pool).await {
        Ok(()) => run_and_count(&maintenance, &pool).await,
        Err(error) => Err(error),
    };
    consumer.close().await?;
    slot_gone(&pool, &live).await?;
    let after_live = (progress(&handle), skipped(&handle, "replication_lag"));
    let resumed = run_and_count(&maintenance, &pool).await?;
    let after_resume = progress(&handle);

    assert_eq!(capped_run, (300, 300), "one run stops at its cap");
    assert_eq!(after_cap.1, (Some(1.0), Some(1.0)));
    assert!(after_cap.0.0.is_some() && after_cap.0.1.is_some());
    assert_eq!(
        past_idle?,
        (200, 200),
        "an inactive slot does not hold the cleanup"
    );
    assert!(after_idle.0 > after_cap.0.0 && after_idle.1 > after_cap.0.1);
    assert_eq!(
        behind_live?,
        (500, 500),
        "an active lagging slot stops the run"
    );
    assert_eq!(after_live.1, (Some(1.0), Some(1.0)));
    assert_eq!(after_live.0, after_idle, "a skipped run is not progress");
    assert_eq!(resumed, (400, 400), "the run goes on once the slot is gone");
    assert!(after_resume.0 > after_idle.0 && after_resume.1 > after_idle.1);
    Ok(())
}
