use std::time::{Duration, Instant};

use sqlx::PgPool;
use tracing::{info, warn};

use crate::metrics;
use crate::queue::{JobError, JobResult};

const TABLES: [&str; 7] = [
    "cmd_reassert",
    "cmd_purge",
    "cmd_done",
    "cmd_dead",
    "repl_purges",
    "repl_tombstones",
    "storage_event_repairs",
];
const BATCH: i32 = 1_000;
const PAUSE: Duration = Duration::from_millis(500);
const BUDGET: Duration = Duration::from_secs(240);

struct Pass {
    deleted: u64,
    held: Option<String>,
    reason: Option<String>,
}

pub(super) async fn run(pool: &PgPool) -> JobResult {
    let started = Instant::now();
    let redacted = redact_dead_letters(pool, started).await?;
    for table in TABLES {
        let pass = delete_old_rows(pool, table, started).await?;
        let redacted = if table == "cmd_dead" { redacted } else { 0 };
        report(table, &pass, redacted);
    }
    Ok(())
}

async fn redact_dead_letters(pool: &PgPool, started: Instant) -> Result<u64, JobError> {
    let mut redacted = 0_u64;
    while started.elapsed() < BUDGET {
        let batch = sqlx::query_file_scalar!("queries/repl/dead_redact.sql", BATCH)
            .fetch_one(pool)
            .await
            .map_err(JobError::retryable)?;
        let batch = u64::try_from(batch).unwrap_or(0);
        redacted += batch;
        if batch < u64::from(BATCH.unsigned_abs()) {
            break;
        }
        tokio::time::sleep(PAUSE).await;
    }
    Ok(redacted)
}

async fn delete_old_rows(
    pool: &PgPool,
    table: &'static str,
    started: Instant,
) -> Result<Pass, JobError> {
    let mut pass = Pass {
        deleted: 0,
        held: None,
        reason: None,
    };
    while started.elapsed() < BUDGET {
        let batch = sqlx::query_file!("queries/repl/retention_batch.sql", table, BATCH)
            .fetch_one(pool)
            .await
            .map_err(JobError::retryable)?;
        let deleted = u64::try_from(batch.deleted).unwrap_or(0);
        pass.deleted += deleted;
        pass.held = batch.held;
        pass.reason = batch.reason;
        if pass.held.is_some() || deleted < u64::from(BATCH.unsigned_abs()) {
            break;
        }
        tokio::time::sleep(PAUSE).await;
    }
    Ok(pass)
}

fn report(table: &'static str, pass: &Pass, redacted: u64) {
    metrics::record_repl_retention(table, pass.deleted, redacted, pass.reason.as_deref());
    match &pass.held {
        Some(reason) => warn!(table, reason, "replication retention held back"),
        None if pass.deleted + redacted > 0 => info!(
            table,
            deleted = pass.deleted,
            redacted,
            "replication retention deleted"
        ),
        None => {}
    }
}
