#[cfg(test)]
#[path = "track_search_tests.rs"]
mod tests;

use uuid::Uuid;

use super::{AdminMaintenanceHandler, PhaseOutcome, RunState, SliceProgress, TRACK_SEARCH};
use crate::queue::{JobError, JobResult};

pub const FIRST_PHASE: &str = "backfill";

const BACKFILL: &str = "backfill";
const CHECK: &str = "check";
const RECHECK: &str = "recheck";
const CHECK_ROWS: i64 = 5_000;
const MAX_LAG_SECONDS: f64 = 10.0;
const MAX_BACKLOG_BYTES: i64 = 1 << 30;
const LOCK_NOT_AVAILABLE: &str = "55P03";
const DEADLOCK_DETECTED: &str = "40P01";
const LAG: &str = include_str!("../../../queries/admin_maintenance/track_search_lag.sql");

#[derive(Debug, sqlx::FromRow)]
struct Lag {
    lag_seconds: f64,
    backlog_bytes: i64,
}

pub async fn execute(
    handler: &AdminMaintenanceHandler,
    state: &RunState,
) -> JobResult<PhaseOutcome> {
    let cursor = state.cursor_uuid.unwrap_or(Uuid::nil());
    if state.phase != RECHECK && lagging(handler).await? {
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
        "queries/admin_maintenance/track_search_backfill.sql",
        cursor,
        handler.track_search_batch
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
        .save_progress(TRACK_SEARCH, state, Some(last), None, progress)
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
        "queries/admin_maintenance/track_search_check.sql",
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
            "track_search differs from tracks"
        );
    }
    if repair && mismatched > 0 {
        let mut transaction = yielding(handler).await?;
        let repaired = sqlx::query_file!(
            "queries/admin_maintenance/track_search_repair.sql",
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
            TRACK_SEARCH,
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
    let totals = sqlx::query_file!("queries/admin_maintenance/load_run.sql", TRACK_SEARCH)
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
            "track_search check finished with mismatches"
        );
    } else {
        tracing::info!(
            scanned = totals.scanned,
            inserted = totals.changed,
            repaired = totals.merged,
            left = 0,
            "track_search check finished clean"
        );
    }
    Ok(PhaseOutcome::Finished)
}

async fn lagging(handler: &AdminMaintenanceHandler) -> JobResult<bool> {
    let lag: Lag = sqlx::query_as(LAG)
        .fetch_one(&handler.pool)
        .await
        .map_err(JobError::retryable)?;
    let lagging = lag.lag_seconds > MAX_LAG_SECONDS || lag.backlog_bytes > MAX_BACKLOG_BYTES;
    if lagging {
        tracing::info!(
            lag_seconds = lag.lag_seconds,
            backlog_mb = lag.backlog_bytes >> 20,
            "track_search waits for the subscribers"
        );
    }
    Ok(lagging)
}

async fn yielding(
    handler: &AdminMaintenanceHandler,
) -> JobResult<sqlx::Transaction<'static, sqlx::Postgres>> {
    let mut transaction = handler.pool.begin().await.map_err(JobError::retryable)?;
    sqlx::query_file!("queries/admin_maintenance/track_search_lock_timeout.sql")
        .execute(&mut *transaction)
        .await
        .map_err(JobError::retryable)?;
    Ok(transaction)
}

fn yielded(error: &sqlx::Error) -> bool {
    error
        .as_database_error()
        .and_then(|error| error.code())
        .is_some_and(|code| code == LOCK_NOT_AVAILABLE || code == DEADLOCK_DETECTED)
}

fn waited(state: &RunState, cursor: Uuid) -> PhaseOutcome {
    tracing::info!(
        phase = %state.phase,
        %cursor,
        "track_search slice yielded to a live writer"
    );
    PhaseOutcome::Advanced
}

async fn advance(
    handler: &AdminMaintenanceHandler,
    state: &RunState,
    phase: &str,
) -> JobResult<PhaseOutcome> {
    handler.advance_phase(TRACK_SEARCH, state, phase).await?;
    Ok(PhaseOutcome::Advanced)
}
