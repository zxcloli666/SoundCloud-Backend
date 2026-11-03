use catalog_normalize::title_forms;
use uuid::Uuid;

use super::{AdminMaintenanceHandler, PhaseOutcome, RunState, SliceProgress};
use crate::queue::{JobError, JobResult};

pub const FIRST_PHASE: &str = "aliases";

const BATCH: i64 = 1000;

pub async fn execute(
    handler: &AdminMaintenanceHandler,
    state: &RunState,
) -> JobResult<PhaseOutcome> {
    let cursor = state.cursor_uuid.unwrap_or(Uuid::nil());
    let rows = sqlx::query_file!(
        "queries/admin_maintenance/wanted_aliases_scan.sql",
        cursor,
        BATCH
    )
    .fetch_all(&handler.pool)
    .await
    .map_err(JobError::retryable)?;

    let Some(tail) = rows.last().map(|row| row.id) else {
        return Ok(PhaseOutcome::Finished);
    };

    let mut wanted_ids = Vec::new();
    let mut alias_wanted_ids = Vec::new();
    let mut alias_keys = Vec::new();
    for row in &rows {
        let mut expected = title_forms(&row.title).aliases;
        expected.sort();
        let mut stored = row.aliases.clone();
        stored.sort();
        if expected == stored {
            continue;
        }
        wanted_ids.push(row.id);
        for alias in expected {
            alias_wanted_ids.push(row.id);
            alias_keys.push(alias);
        }
    }

    if !wanted_ids.is_empty() {
        sqlx::query_file!(
            "queries/crawl/replace_wanted_aliases_batch.sql",
            &wanted_ids,
            &alias_wanted_ids,
            &alias_keys
        )
        .execute(&handler.pool)
        .await
        .map_err(JobError::retryable)?;
    }

    let progress = SliceProgress {
        scanned: rows.len() as i64,
        changed: wanted_ids.len() as i64,
        ..SliceProgress::default()
    };
    handler
        .save_progress(super::WANTED_ALIASES, state, Some(tail), None, progress)
        .await?;
    Ok(PhaseOutcome::Advanced)
}
