#[cfg(test)]
#[path = "track_sync_stamps_tests.rs"]
mod tests;

use uuid::Uuid;

use super::track_search::{held_back, yielded, yielding};
use super::{AdminMaintenanceHandler, PhaseOutcome, RunState, SliceProgress, TRACK_SYNC_STAMPS};
use crate::queue::{JobError, JobResult};

pub const FIRST_PHASE: &str = "backfill";

const BACKFILL: &str = "backfill";
const CHECK: &str = "check";
const RECHECK: &str = "recheck";
const CHECK_ROWS: i64 = 20_000;
const MAX_LAG_SECONDS: f64 = 60.0;

pub async fn execute(
    handler: &AdminMaintenanceHandler,
    state: &RunState,
) -> JobResult<PhaseOutcome> {
    let cursor = state.cursor_uuid.unwrap_or(Uuid::nil());
    if state.phase != RECHECK && held_back(handler, MAX_LAG_SECONDS).await? {
        return Ok(PhaseOutcome::Advanced);
    }
    match state.phase.as_str() {
        BACKFILL => backfill(handler, state, cursor).await,
        CHECK => check(handler, state, cursor, true).await,
        RECHECK => check(handler, state, cursor, false).await,
        _ => Ok(PhaseOutcome::Finished),
    }
}

async fn backfill(
    handler: &AdminMaintenanceHandler,
    state: &RunState,
    cursor: Uuid,
) -> JobResult<PhaseOutcome> {
    let mut transaction = yielding(handler).await?;
    let batch = sqlx::query_file!(
        "queries/admin_maintenance/track_sync_stamps_backfill.sql",
        cursor,
        handler.track_sync_stamps_batch
    )
    .fetch_one(&mut *transaction)
    .await;
    let batch = match batch {
        Ok(batch) => batch,
        Err(error) if yielded(&error) => return Ok(waited(state, cursor)),
        Err(error) => return Err(JobError::retryable(error)),
    };
    transaction.commit().await.map_err(JobError::retryable)?;
    let Some(last) = batch.last else {
        return advance(handler, state, CHECK).await;
    };
    let progress = SliceProgress {
        scanned: batch.scanned,
        changed: batch.inserted,
        ..SliceProgress::default()
    };
    handler
        .save_progress(TRACK_SYNC_STAMPS, state, Some(last), None, progress)
        .await?;
    Ok(PhaseOutcome::Advanced)
}

async fn check(
    handler: &AdminMaintenanceHandler,
    state: &RunState,
    cursor: Uuid,
    repair: bool,
) -> JobResult<PhaseOutcome> {
    let batch = sqlx::query_file!(
        "queries/admin_maintenance/track_sync_stamps_check.sql",
        cursor,
        CHECK_ROWS
    )
    .fetch_one(&handler.pool)
    .await
    .map_err(JobError::retryable)?;
    let mismatched = batch.mismatched.len() as i64;
    if mismatched > 0 {
        tracing::warn!(
            phase = %state.phase,
            %cursor,
            mismatched,
            sample = ?batch.mismatched.iter().take(5).collect::<Vec<_>>(),
            "track_sync_stamps is missing or behind tracks"
        );
    }
    if repair && mismatched > 0 {
        let mut transaction = yielding(handler).await?;
        let repaired = sqlx::query_file!(
            "queries/admin_maintenance/track_sync_stamps_repair.sql",
            &batch.mismatched
        )
        .fetch_one(&mut *transaction)
        .await;
        match repaired {
            Ok(_) => transaction.commit().await.map_err(JobError::retryable)?,
            Err(error) if yielded(&error) => return Ok(waited(state, cursor)),
            Err(error) => return Err(JobError::retryable(error)),
        }
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
            TRACK_SYNC_STAMPS,
            state,
            batch.last.or(Some(cursor)),
            None,
            progress,
        )
        .await?;
    if batch.last.is_none() {
        return finish(handler, state).await;
    }
    Ok(PhaseOutcome::Advanced)
}

async fn finish(handler: &AdminMaintenanceHandler, state: &RunState) -> JobResult<PhaseOutcome> {
    let totals = sqlx::query_file!("queries/admin_maintenance/load_run.sql", TRACK_SYNC_STAMPS)
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
            "track_sync_stamps check finished with mismatches"
        );
    } else {
        tracing::info!(
            scanned = totals.scanned,
            inserted = totals.changed,
            repaired = totals.merged,
            left = 0,
            "track_sync_stamps check finished clean"
        );
    }
    Ok(PhaseOutcome::Finished)
}

fn waited(state: &RunState, cursor: Uuid) -> PhaseOutcome {
    tracing::info!(
        phase = %state.phase,
        %cursor,
        "track_sync_stamps slice yielded to a live writer"
    );
    PhaseOutcome::Advanced
}

async fn advance(
    handler: &AdminMaintenanceHandler,
    state: &RunState,
    phase: &str,
) -> JobResult<PhaseOutcome> {
    handler
        .advance_phase(TRACK_SYNC_STAMPS, state, phase)
        .await?;
    Ok(PhaseOutcome::Advanced)
}
