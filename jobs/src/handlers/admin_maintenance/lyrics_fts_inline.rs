#[cfg(test)]
#[path = "lyrics_fts_inline_tests.rs"]
mod tests;

use super::track_search::{MAX_LAG_SECONDS, held_back, yielded, yielding};
use super::{AdminMaintenanceHandler, LYRICS_FTS_INLINE, PhaseOutcome, RunState, SliceProgress};
use crate::queue::{JobError, JobResult};

pub const FIRST_PHASE: &str = "inline";

const MAX_INLINE_BYTES: i32 = 7_000;

pub async fn execute(
    handler: &AdminMaintenanceHandler,
    state: &RunState,
) -> JobResult<PhaseOutcome> {
    if held_back(handler, MAX_LAG_SECONDS).await? {
        return Ok(PhaseOutcome::Advanced);
    }
    let cursor = state.cursor_text.as_deref().unwrap_or("");
    let mut transaction = yielding(handler).await?;
    let batch = sqlx::query_file!(
        "queries/admin_maintenance/lyrics_fts_inline.sql",
        cursor,
        handler.lyrics_fts_inline_batch,
        MAX_INLINE_BYTES
    )
    .fetch_one(&mut *transaction)
    .await;
    let batch = match batch {
        Ok(batch) => batch,
        Err(error) if yielded(&error) => {
            tracing::info!(cursor, "lyrics_fts_inline slice yielded to a live writer");
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
        changed: batch.moved,
        ..SliceProgress::default()
    };
    handler
        .save_progress(LYRICS_FTS_INLINE, state, None, Some(&last), progress)
        .await?;
    Ok(PhaseOutcome::Advanced)
}
