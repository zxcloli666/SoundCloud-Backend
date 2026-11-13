#[cfg(test)]
#[path = "lyrics_description_tests.rs"]
mod tests;

use super::track_search::{lagging, yielded, yielding};
use super::{AdminMaintenanceHandler, LYRICS_DESCRIPTION, PhaseOutcome, RunState, SliceProgress};
use crate::queue::{JobError, JobResult};

pub const FIRST_PHASE: &str = "strip";

pub async fn execute(
    handler: &AdminMaintenanceHandler,
    state: &RunState,
) -> JobResult<PhaseOutcome> {
    if lagging(handler).await? {
        return Ok(PhaseOutcome::Advanced);
    }
    let cursor = state.cursor_text.as_deref().unwrap_or("");
    let mut transaction = yielding(handler).await?;
    let batch = sqlx::query_file!(
        "queries/admin_maintenance/lyrics_description_strip.sql",
        cursor,
        handler.track_search_batch
    )
    .fetch_one(&mut *transaction)
    .await;
    let batch = match batch {
        Ok(batch) => batch,
        Err(error) if yielded(&error) => {
            tracing::info!(cursor, "lyrics_description slice yielded to a live writer");
            return Ok(PhaseOutcome::Advanced);
        }
        Err(error) => return Err(JobError::retryable(error)),
    };
    transaction.commit().await.map_err(JobError::retryable)?;
    let Some(last) = batch.last else {
        return Ok(PhaseOutcome::Finished);
    };
    let progress = SliceProgress {
        scanned: batch.scanned,
        changed: batch.stripped,
        ..SliceProgress::default()
    };
    handler
        .save_progress(LYRICS_DESCRIPTION, state, None, Some(&last), progress)
        .await?;
    Ok(PhaseOutcome::Advanced)
}
