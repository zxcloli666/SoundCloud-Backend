#[cfg(test)]
#[path = "lyrics_queue_tests.rs"]
mod tests;

use super::{AdminMaintenanceHandler, LYRICS_LOOKUP_QUEUE, PhaseOutcome, RunState, SliceProgress};
use crate::queue::{JobError, JobResult};

pub const FIRST_PHASE: &str = "backfill";

const BACKFILL: &str = "backfill";
const CHECK: &str = "check";
const RECHECK: &str = "recheck";
const PAGES: i64 = 256;

pub async fn execute(
    handler: &AdminMaintenanceHandler,
    state: &RunState,
) -> JobResult<PhaseOutcome> {
    let start = state
        .cursor_text
        .as_deref()
        .and_then(|page| page.parse::<i64>().ok())
        .unwrap_or(0);
    let pages = sqlx::query_file_scalar!("queries/admin_maintenance/lyrics_queue_pages.sql")
        .fetch_one(&handler.pool)
        .await
        .map_err(JobError::retryable)?;
    let passed = start >= pages;
    match state.phase.as_str() {
        BACKFILL if passed => advance(handler, state, CHECK).await,
        BACKFILL => backfill(handler, state, start).await,
        CHECK | RECHECK if passed => finish(handler, state).await,
        CHECK => check(handler, state, start, true).await,
        RECHECK => check(handler, state, start, false).await,
        _ => Ok(PhaseOutcome::Finished),
    }
}

async fn backfill(
    handler: &AdminMaintenanceHandler,
    state: &RunState,
    start: i64,
) -> JobResult<PhaseOutcome> {
    let end = start + PAGES;
    let batch = sqlx::query_file!(
        "queries/admin_maintenance/lyrics_queue_backfill.sql",
        tid(start),
        tid(end)
    )
    .fetch_one(&handler.pool)
    .await
    .map_err(JobError::retryable)?;
    let progress = SliceProgress {
        scanned: batch.scanned,
        changed: batch.inserted,
        ..SliceProgress::default()
    };
    handler
        .save_progress(
            LYRICS_LOOKUP_QUEUE,
            state,
            None,
            Some(&end.to_string()),
            progress,
        )
        .await?;
    Ok(PhaseOutcome::Advanced)
}

async fn check(
    handler: &AdminMaintenanceHandler,
    state: &RunState,
    start: i64,
    repair: bool,
) -> JobResult<PhaseOutcome> {
    let end = start + PAGES;
    let batch = sqlx::query_file!(
        "queries/admin_maintenance/lyrics_queue_check.sql",
        tid(start),
        tid(end)
    )
    .fetch_one(&handler.pool)
    .await
    .map_err(JobError::retryable)?;
    let mismatched = batch.mismatched.len() as i64;
    if mismatched > 0 {
        tracing::warn!(
            phase = %state.phase,
            start,
            mismatched,
            sample = ?batch.mismatched.iter().take(5).collect::<Vec<_>>(),
            "lyrics_lookup_queue differs from lyrics_lookup_state"
        );
    }
    if repair && mismatched > 0 {
        sqlx::query_file!(
            "queries/admin_maintenance/lyrics_queue_repair.sql",
            &batch.mismatched
        )
        .execute(&handler.pool)
        .await
        .map_err(JobError::retryable)?;
    }
    let progress = if repair {
        SliceProgress {
            scanned: batch.scanned,
            merged: mismatched,
            ..SliceProgress::default()
        }
    } else {
        SliceProgress {
            scanned: batch.scanned,
            skipped: mismatched,
            ..SliceProgress::default()
        }
    };
    handler
        .save_progress(
            LYRICS_LOOKUP_QUEUE,
            state,
            None,
            Some(&end.to_string()),
            progress,
        )
        .await?;
    Ok(PhaseOutcome::Advanced)
}

async fn finish(handler: &AdminMaintenanceHandler, state: &RunState) -> JobResult<PhaseOutcome> {
    let totals = sqlx::query_file!(
        "queries/admin_maintenance/load_run.sql",
        LYRICS_LOOKUP_QUEUE
    )
    .fetch_one(&handler.pool)
    .await
    .map_err(JobError::retryable)?;
    if state.phase == CHECK && totals.merged > 0 {
        return advance(handler, state, RECHECK).await;
    }
    if totals.skipped > 0 {
        tracing::error!(
            scanned = totals.scanned,
            inserted = totals.changed,
            repaired = totals.merged,
            left = totals.skipped,
            "lyrics_lookup_queue check finished with mismatches"
        );
    } else {
        tracing::info!(
            scanned = totals.scanned,
            inserted = totals.changed,
            repaired = totals.merged,
            left = 0,
            "lyrics_lookup_queue check finished clean"
        );
    }
    Ok(PhaseOutcome::Finished)
}

async fn advance(
    handler: &AdminMaintenanceHandler,
    state: &RunState,
    phase: &str,
) -> JobResult<PhaseOutcome> {
    handler
        .advance_phase(LYRICS_LOOKUP_QUEUE, state, phase)
        .await?;
    Ok(PhaseOutcome::Advanced)
}

fn tid(page: i64) -> String {
    format!("({page},0)")
}
