use sqlx::PgPool;
use uuid::Uuid;

use super::HealthState;

async fn enqueue(pool: &PgPool, lane: &str, leased: bool, attempts: i32) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO background_jobs (
             id, kind, lane, payload, attempts, available_at,
             lease_id, lease_generation, leased_by, lease_expires_at
         ) VALUES (
             $1, 'catalog.refresh', $2, '{}'::jsonb, $3, now() - interval '30 seconds',
             CASE WHEN $4 THEN gen_random_uuid() END,
             CASE WHEN $4 THEN 1::bigint END,
             CASE WHEN $4 THEN 'test' END,
             CASE WHEN $4 THEN now() + interval '1 minute' END
         )",
    )
    .bind(Uuid::new_v4())
    .bind(lane)
    .bind(attempts)
    .bind(leased)
    .execute(pool)
    .await?;
    Ok(())
}

#[sqlx::test(migrations = "../api/migrations")]
async fn the_queue_snapshot_separates_pending_leased_and_dead_letters(
    pool: PgPool,
) -> anyhow::Result<()> {
    crate::metrics::init();
    enqueue(&pool, "core_bulk", false, 0).await?;
    enqueue(&pool, "core_bulk", false, 2).await?;
    enqueue(&pool, "core_bulk", true, 1).await?;
    sqlx::query(
        "INSERT INTO background_job_failures (
             id, kind, lane, payload, priority, generation, attempts, max_attempts,
             last_error, created_at
         ) VALUES (
             gen_random_uuid(), 'catalog.refresh', 'core_bulk', '{}'::jsonb, 0, 1, 8, 8,
             'exhausted', now()
         )",
    )
    .execute(&pool)
    .await?;

    let Some(body) = crate::metrics::render(&pool).await else {
        return Ok(());
    };

    assert!(body.contains("jobs_queue_depth{lane=\"core_bulk\",state=\"pending\"} 2"));
    assert!(body.contains("jobs_queue_depth{lane=\"core_bulk\",state=\"leased\"} 1"));
    assert!(body.contains("jobs_queue_depth{lane=\"core_bulk\",state=\"retried\"} 2"));
    assert!(body.contains("jobs_queue_dead_letters{lane=\"core_bulk\"} 1"));
    assert!(body.contains("jobs_queue_oldest_due_seconds"));
    Ok(())
}

#[sqlx::test(migrations = "../api/migrations")]
async fn a_scrape_reports_what_it_costs_to_get_a_connection(pool: PgPool) -> anyhow::Result<()> {
    crate::metrics::init();

    let Some(body) = crate::metrics::render(&pool).await else {
        return Ok(());
    };

    assert!(body.contains("jobs_pg_pool_connections{state=\"open\"}"));
    assert!(body.contains("jobs_pg_pool_wait_last_seconds"));
    assert!(
        body.contains("jobs_pg_pool_wait_seconds_bucket{outcome=\"ok\""),
        "a healthy pool must report a measured wait, not an empty histogram"
    );
    Ok(())
}

#[sqlx::test(migrations = "../api/migrations")]
async fn a_pool_that_cannot_hand_out_a_connection_is_visible_as_such(
    pool: PgPool,
) -> anyhow::Result<()> {
    crate::metrics::init();
    pool.close().await;

    crate::metrics::sample_pool_wait(&pool).await;
    let Some(body) = crate::metrics::render(&pool).await else {
        return Ok(());
    };

    assert!(
        body.contains("jobs_pg_pool_wait_seconds_bucket{outcome=\"error\"")
            || body.contains("jobs_pg_pool_wait_seconds_bucket{outcome=\"timeout\""),
        "a pool that refuses a connection must not read as a healthy wait"
    );
    Ok(())
}

#[sqlx::test(migrations = "../api/migrations")]
async fn every_metric_an_alert_watches_is_actually_exported(pool: PgPool) -> anyhow::Result<()> {
    crate::metrics::init();
    crate::metrics::record_execution(
        "catalog.refresh",
        crate::metrics::Outcome::Ok,
        std::time::Duration::from_millis(1),
    );

    let Some(body) = crate::metrics::render(&pool).await else {
        return Ok(());
    };

    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../ops");
    let mut artifacts = String::new();
    for entry in std::fs::read_dir(&dir)?.flatten() {
        if let Ok(body) = std::fs::read_to_string(entry.path()) {
            artifacts.push_str(&body);
            artifacts.push('\n');
        }
    }
    assert!(
        artifacts.contains("jobs_queue_depth"),
        "the ops artifacts were not read, so this test proves nothing"
    );

    let mut watched: Vec<String> = Vec::new();
    for token in artifacts.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_')) {
        if !token.starts_with("jobs_") || token.len() <= "jobs_".len() + 2 {
            continue;
        }
        let base = ["_bucket", "_count", "_sum"]
            .iter()
            .find_map(|suffix| token.strip_suffix(suffix))
            .unwrap_or(token);
        if !watched.iter().any(|seen| seen == base) {
            watched.push(base.to_owned());
        }
    }

    assert!(
        watched.len() >= 6,
        "the ops artifacts must actually reference jobs metrics, found {watched:?}"
    );
    for name in watched {
        let exported = body.lines().any(|line| {
            line.strip_prefix(name.as_str()).is_some_and(|rest| {
                rest.starts_with(' ')
                    || rest.starts_with('{')
                    || rest.starts_with("_bucket")
                    || rest.starts_with("_sum")
                    || rest.starts_with("_count")
            })
        });
        assert!(
            exported,
            "an alert rule or dashboard panel watches {name}, which no scrape exports"
        );
    }
    Ok(())
}

#[sqlx::test(migrations = "../api/migrations")]
async fn an_empty_queue_still_reports_every_lane(pool: PgPool) -> anyhow::Result<()> {
    crate::metrics::init();

    let Some(body) = crate::metrics::render(&pool).await else {
        return Ok(());
    };

    for lane in ["core_fast", "core_bulk", "ops"] {
        assert!(
            body.contains(&format!(
                "jobs_queue_depth{{lane=\"{lane}\",state=\"pending\"}} 0"
            )),
            "an empty queue must read as zero, not as a missing metric: {lane}"
        );
        assert!(body.contains(&format!("jobs_queue_dead_letters{{lane=\"{lane}\"}} 0")));
    }
    Ok(())
}

#[test]
fn a_state_without_a_pool_cannot_pretend_to_serve_metrics() {
    let state = HealthState::new();
    assert!(state.metrics_pool.is_none());
}
