#[cfg(test)]
#[path = "lyrics_refetch_tests.rs"]
mod tests;

use super::track_search::{lagging, yielded, yielding};
use super::{AdminMaintenanceHandler, LYRICS_REFETCH, PhaseOutcome, RunState, SliceProgress};
use crate::queue::{JobError, JobResult};

pub const FIRST_PHASE: &str = "seed";

const SEED: &str = "seed";
const DRAIN: &str = "drain";

pub async fn execute(
    handler: &AdminMaintenanceHandler,
    state: &RunState,
) -> JobResult<PhaseOutcome> {
    match state.phase.as_str() {
        SEED => seed(handler, state).await,
        DRAIN => drain(handler, state).await,
        _ => Ok(PhaseOutcome::Finished),
    }
}

async fn seed(handler: &AdminMaintenanceHandler, state: &RunState) -> JobResult<PhaseOutcome> {
    if lagging(handler).await? {
        return Ok(PhaseOutcome::Advanced);
    }
    let cursor = state.cursor_text.as_deref().unwrap_or("");
    let mut transaction = yielding(handler).await?;
    let batch = sqlx::query_file!(
        "queries/admin_maintenance/lyrics_refetch_seed.sql",
        cursor,
        handler.track_search_batch
    )
    .fetch_one(&mut *transaction)
    .await;
    let batch = match batch {
        Ok(batch) => batch,
        Err(error) if yielded(&error) => {
            tracing::info!(cursor, "lyrics_refetch seed yielded to a live writer");
            return Ok(PhaseOutcome::Advanced);
        }
        Err(error) => return Err(JobError::retryable(error)),
    };
    transaction.commit().await.map_err(JobError::retryable)?;
    let Some(last) = batch.last else {
        handler.advance_phase(LYRICS_REFETCH, state, DRAIN).await?;
        return Ok(PhaseOutcome::Advanced);
    };
    let progress = SliceProgress {
        scanned: batch.armed,
        ..SliceProgress::default()
    };
    handler
        .save_progress(LYRICS_REFETCH, state, None, Some(&last), progress)
        .await?;
    Ok(PhaseOutcome::Advanced)
}

async fn drain(handler: &AdminMaintenanceHandler, state: &RunState) -> JobResult<PhaseOutcome> {
    let counts = sqlx::query_file!("queries/admin_maintenance/lyrics_refetch_progress.sql")
        .fetch_one(&handler.pool)
        .await
        .map_err(JobError::retryable)?;
    sqlx::query_file!(
        "queries/admin_maintenance/lyrics_refetch_set_progress.sql",
        LYRICS_REFETCH,
        state.run_id,
        counts.armed,
        counts.replaced,
        counts.kept,
        counts.lost
    )
    .execute(&handler.pool)
    .await
    .map_err(JobError::retryable)?;
    crate::metrics::set_lyrics_refetch_pending(counts.pending);
    if counts.pending > 0 {
        return Ok(PhaseOutcome::Waiting);
    }
    tracing::info!(
        armed = counts.armed,
        replaced = counts.replaced,
        kept = counts.kept,
        lost = counts.lost,
        "lyrics_refetch finished"
    );
    Ok(PhaseOutcome::Finished)
}
