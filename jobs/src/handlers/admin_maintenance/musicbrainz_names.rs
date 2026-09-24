use catalog_normalize::{compact_key, normalize_name};
use uuid::Uuid;

use super::renormalize::{is_unique_violation, repoint_references};
use super::{AdminMaintenanceHandler, MUSICBRAINZ_NAMES, PhaseOutcome, RunState, SliceProgress};
use crate::queue::{JobError, JobResult};

pub const FIRST_PHASE: &str = "names";

const NAMES_BATCH: i64 = 200;

pub async fn execute(
    handler: &AdminMaintenanceHandler,
    state: &RunState,
) -> JobResult<PhaseOutcome> {
    let cursor = state.cursor_uuid.unwrap_or(Uuid::nil());
    let rows = sqlx::query_file!(
        "queries/admin_maintenance/artists_mb_scan.sql",
        cursor,
        NAMES_BATCH
    )
    .fetch_all(&handler.pool)
    .await
    .map_err(JobError::retryable)?;

    let Some(tail) = rows.last().map(|row| row.id) else {
        return Ok(PhaseOutcome::Finished);
    };

    let mut progress = SliceProgress {
        scanned: rows.len() as i64,
        ..SliceProgress::default()
    };
    for row in &rows {
        let details = match handler.musicbrainz.lookup_artist(&row.mb_artist_id).await {
            Ok(Some(details)) => details,
            Ok(None) => {
                progress.skipped += 1;
                continue;
            }
            Err(error) => return Err(JobError::retryable(error)),
        };
        let Some(entity) = details.name else {
            progress.skipped += 1;
            continue;
        };
        let fresh = normalize_name(&entity);
        if fresh.is_empty() || fresh == row.normalized_name {
            continue;
        }
        let stored = compact_key(&row.name);
        let candidate = compact_key(&entity);
        if stored.is_empty() || !candidate.contains(&stored) {
            progress.skipped += 1;
            continue;
        }

        let holder: Option<Uuid> = sqlx::query_file_scalar!(
            "queries/admin_maintenance/artist_normalized_holder.sql",
            fresh,
            row.id
        )
        .fetch_optional(&handler.pool)
        .await
        .map_err(JobError::retryable)?;

        match holder {
            Some(holder) => {
                merge_alias(
                    handler,
                    row.id,
                    &row.mb_artist_id,
                    row.genius_artist_id.as_deref(),
                    holder,
                )
                .await?;
                progress.merged += 1;
            }
            None => {
                let renamed = sqlx::query_file!(
                    "queries/admin_maintenance/artist_set_name.sql",
                    row.id,
                    &entity,
                    fresh
                )
                .execute(&handler.pool)
                .await;
                match renamed {
                    Ok(_) => progress.changed += 1,
                    Err(error) if is_unique_violation(&error) => {
                        let holder: Option<Uuid> = sqlx::query_file_scalar!(
                            "queries/admin_maintenance/artist_normalized_holder.sql",
                            fresh,
                            row.id
                        )
                        .fetch_optional(&handler.pool)
                        .await
                        .map_err(JobError::retryable)?;
                        if let Some(holder) = holder {
                            merge_alias(
                                handler,
                                row.id,
                                &row.mb_artist_id,
                                row.genius_artist_id.as_deref(),
                                holder,
                            )
                            .await?;
                            progress.merged += 1;
                        }
                    }
                    Err(error) => return Err(JobError::retryable(error)),
                }
            }
        }
    }

    handler
        .save_progress(MUSICBRAINZ_NAMES, state, Some(tail), None, progress)
        .await?;
    Ok(PhaseOutcome::Advanced)
}

async fn merge_alias(
    handler: &AdminMaintenanceHandler,
    alias: Uuid,
    mb_artist_id: &str,
    genius_artist_id: Option<&str>,
    holder: Uuid,
) -> JobResult {
    sqlx::query_file!(
        "queries/admin_maintenance/artist_mark_merged_clear_ids.sql",
        alias,
        holder
    )
    .execute(&handler.pool)
    .await
    .map_err(JobError::retryable)?;
    sqlx::query_file!(
        "queries/admin_maintenance/artist_fill_external_ids.sql",
        holder,
        mb_artist_id,
        genius_artist_id
    )
    .execute(&handler.pool)
    .await
    .map_err(JobError::retryable)?;
    repoint_references(handler, alias, holder).await
}
