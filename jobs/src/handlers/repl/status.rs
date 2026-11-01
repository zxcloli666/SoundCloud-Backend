use std::sync::Mutex;
use std::time::{Duration, Instant};

use sqlx::PgPool;
use tracing::info;

use crate::queue::{JobError, JobResult};

const BATCH: i32 = 1_000;
const BACKFILL_EVERY: Duration = Duration::from_secs(60 * 60);

pub(super) async fn reconcile(pool: &PgPool, backfilled_at: &Mutex<Option<Instant>>) -> JobResult {
    let backfill_due = backfilled_at.lock().map_or(true, |at| {
        at.is_none_or(|at| at.elapsed() >= BACKFILL_EVERY)
    });
    let mut added = 0;
    if backfill_due {
        let mut after = None;
        loop {
            let page = sqlx::query_file!(
                "queries/repl/job_status_backfill.sql",
                after.as_ref(),
                BATCH
            )
            .fetch_one(pool)
            .await
            .map_err(JobError::retryable)?;
            added += page.added;
            match page.last_id {
                Some(last) => after = Some(last),
                None => break,
            }
        }
        if let Ok(mut at) = backfilled_at.lock() {
            *at = Some(Instant::now());
        }
    }
    let mut removed = 0;
    loop {
        let pruned = sqlx::query_file_scalar!("queries/repl/job_status_prune.sql", BATCH)
            .fetch_one(pool)
            .await
            .map_err(JobError::retryable)?;
        removed += pruned;
        if pruned < i64::from(BATCH) {
            break;
        }
    }
    if added > 0 || removed > 0 {
        info!(added, removed, "job status reconciled with the queue");
    }
    Ok(())
}
