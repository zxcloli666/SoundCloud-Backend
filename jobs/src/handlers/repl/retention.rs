use std::time::{Duration, Instant};

use sqlx::PgPool;
use tracing::{info, warn};

use crate::metrics;
use crate::queue::{JobError, JobResult};

const TABLES: [&str; 4] = ["cmd_done", "cmd_dead", "repl_purges", "repl_tombstones"];
const BATCH: i32 = 1_000;
const PAUSE: Duration = Duration::from_millis(500);
const BUDGET: Duration = Duration::from_secs(240);

pub(super) async fn run(pool: &PgPool) -> JobResult {
    let started = Instant::now();
    for table in TABLES {
        let mut deleted_total = 0_u64;
        let mut held = None;
        while started.elapsed() < BUDGET {
            let batch = sqlx::query_file!("queries/repl/retention_batch.sql", table, BATCH)
                .fetch_one(pool)
                .await
                .map_err(JobError::retryable)?;
            held = batch.held;
            let deleted = u64::try_from(batch.deleted).unwrap_or(0);
            deleted_total += deleted;
            if held.is_some() || deleted < u64::from(BATCH.unsigned_abs()) {
                break;
            }
            tokio::time::sleep(PAUSE).await;
        }
        metrics::record_repl_retention(table, deleted_total, held.is_some());
        match held {
            Some(reason) => warn!(table, reason, "replication retention held back"),
            None if deleted_total > 0 => info!(
                table,
                deleted = deleted_total,
                "replication retention deleted"
            ),
            None => {}
        }
    }
    Ok(())
}
