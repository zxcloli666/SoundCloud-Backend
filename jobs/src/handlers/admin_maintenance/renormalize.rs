use catalog_normalize::{normalize_name, normalize_title, unescape_json_unicode};
use uuid::Uuid;

use super::{AdminMaintenanceHandler, CATALOG_RENORMALIZE, PhaseOutcome, RunState, SliceProgress};
use crate::queue::{JobError, JobResult};

pub const FIRST_PHASE: &str = "artists";

const ARTISTS: &str = "artists";
const REPOINT: &str = "repoint";
const ROLES: &str = "roles";
const TRACK_TITLES: &str = "track_titles";
const ALBUM_TITLES: &str = "album_titles";
const PLAYLIST_TITLES: &str = "playlist_titles";
const TRACK_META: &str = "track_meta";

pub async fn execute(
    handler: &AdminMaintenanceHandler,
    state: &RunState,
) -> JobResult<PhaseOutcome> {
    match state.phase.as_str() {
        ARTISTS => artists(handler, state).await,
        REPOINT => repoint(handler, state).await,
        ROLES => roles(handler, state).await,
        TRACK_TITLES => track_titles(handler, state).await,
        ALBUM_TITLES => album_titles(handler, state).await,
        PLAYLIST_TITLES => playlist_titles(handler, state).await,
        TRACK_META => track_meta(handler, state).await,
        _ => Ok(PhaseOutcome::Finished),
    }
}

async fn artists(handler: &AdminMaintenanceHandler, state: &RunState) -> JobResult<PhaseOutcome> {
    let cursor = state.cursor_uuid.unwrap_or(Uuid::nil());
    let rows = sqlx::query_file!(
        "queries/admin_maintenance/artists_scan.sql",
        cursor,
        handler.scan_batch
    )
    .fetch_all(&handler.pool)
    .await
    .map_err(JobError::retryable)?;

    let Some(tail) = rows.last().map(|row| row.id) else {
        handler
            .advance_phase(CATALOG_RENORMALIZE, state, REPOINT)
            .await?;
        return Ok(PhaseOutcome::Advanced);
    };

    let mut progress = SliceProgress {
        scanned: rows.len() as i64,
        ..SliceProgress::default()
    };
    for row in &rows {
        let fresh = normalize_name(&row.name);
        if fresh.is_empty() || fresh == row.normalized_name {
            continue;
        }
        let updated = sqlx::query_file!(
            "queries/admin_maintenance/artist_set_normalized.sql",
            row.id,
            fresh
        )
        .execute(&handler.pool)
        .await;
        match updated {
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
                    sqlx::query_file!(
                        "queries/admin_maintenance/artist_mark_merged.sql",
                        row.id,
                        holder
                    )
                    .execute(&handler.pool)
                    .await
                    .map_err(JobError::retryable)?;
                    progress.merged += 1;
                }
            }
            Err(error) => return Err(JobError::retryable(error)),
        }
    }

    handler
        .save_progress(CATALOG_RENORMALIZE, state, Some(tail), None, progress)
        .await?;
    Ok(PhaseOutcome::Advanced)
}

async fn repoint(handler: &AdminMaintenanceHandler, state: &RunState) -> JobResult<PhaseOutcome> {
    let cursor = state.cursor_uuid.unwrap_or(Uuid::nil());
    let rows = sqlx::query_file!(
        "queries/admin_maintenance/merged_artists_scan.sql",
        cursor,
        handler.scan_batch
    )
    .fetch_all(&handler.pool)
    .await
    .map_err(JobError::retryable)?;

    let Some(tail) = rows.last().map(|row| row.id) else {
        handler
            .advance_phase(CATALOG_RENORMALIZE, state, ROLES)
            .await?;
        return Ok(PhaseOutcome::Advanced);
    };

    let mut progress = SliceProgress {
        scanned: rows.len() as i64,
        ..SliceProgress::default()
    };
    for row in &rows {
        let holder = merge_root(handler, row.merged_into).await?;
        repoint_references(handler, row.id, holder).await?;
        progress.merged += 1;
    }

    handler
        .save_progress(CATALOG_RENORMALIZE, state, Some(tail), None, progress)
        .await?;
    Ok(PhaseOutcome::Advanced)
}

async fn roles(handler: &AdminMaintenanceHandler, state: &RunState) -> JobResult<PhaseOutcome> {
    sqlx::query_file!("queries/admin_maintenance/role_feature_dedup.sql")
        .execute(&handler.pool)
        .await
        .map_err(JobError::retryable)?;
    let updated = sqlx::query_file!("queries/admin_maintenance/role_feature_update.sql")
        .execute(&handler.pool)
        .await
        .map_err(JobError::retryable)?
        .rows_affected();
    handler
        .save_progress(
            CATALOG_RENORMALIZE,
            state,
            None,
            None,
            SliceProgress {
                changed: updated as i64,
                ..SliceProgress::default()
            },
        )
        .await?;
    handler
        .advance_phase(CATALOG_RENORMALIZE, state, TRACK_TITLES)
        .await?;
    Ok(PhaseOutcome::Advanced)
}

async fn track_titles(
    handler: &AdminMaintenanceHandler,
    state: &RunState,
) -> JobResult<PhaseOutcome> {
    let cursor = state.cursor_uuid.unwrap_or(Uuid::nil());
    let rows = sqlx::query_file!(
        "queries/admin_maintenance/tracks_title_scan.sql",
        cursor,
        handler.scan_batch
    )
    .fetch_all(&handler.pool)
    .await
    .map_err(JobError::retryable)?;

    let Some(tail) = rows.last().map(|row| row.id) else {
        handler
            .advance_phase(CATALOG_RENORMALIZE, state, ALBUM_TITLES)
            .await?;
        return Ok(PhaseOutcome::Advanced);
    };

    let mut progress = SliceProgress {
        scanned: rows.len() as i64,
        ..SliceProgress::default()
    };
    for row in &rows {
        let fresh = normalize_title(&row.title);
        if fresh == row.title_normalized {
            continue;
        }
        sqlx::query_file!(
            "queries/admin_maintenance/track_set_title_norm.sql",
            row.id,
            fresh
        )
        .execute(&handler.pool)
        .await
        .map_err(JobError::retryable)?;
        progress.changed += 1;
    }

    handler
        .save_progress(CATALOG_RENORMALIZE, state, Some(tail), None, progress)
        .await?;
    Ok(PhaseOutcome::Advanced)
}

async fn album_titles(
    handler: &AdminMaintenanceHandler,
    state: &RunState,
) -> JobResult<PhaseOutcome> {
    let cursor = state.cursor_uuid.unwrap_or(Uuid::nil());
    let rows = sqlx::query_file!(
        "queries/admin_maintenance/albums_title_scan.sql",
        cursor,
        handler.scan_batch
    )
    .fetch_all(&handler.pool)
    .await
    .map_err(JobError::retryable)?;

    let Some(tail) = rows.last().map(|row| row.id) else {
        handler
            .advance_phase(CATALOG_RENORMALIZE, state, PLAYLIST_TITLES)
            .await?;
        return Ok(PhaseOutcome::Advanced);
    };

    let mut progress = SliceProgress {
        scanned: rows.len() as i64,
        ..SliceProgress::default()
    };
    for row in &rows {
        let fresh = normalize_title(&row.title);
        if fresh == row.normalized_title {
            continue;
        }
        sqlx::query_file!(
            "queries/admin_maintenance/album_set_title_norm.sql",
            row.id,
            fresh
        )
        .execute(&handler.pool)
        .await
        .map_err(JobError::retryable)?;
        progress.changed += 1;
    }

    handler
        .save_progress(CATALOG_RENORMALIZE, state, Some(tail), None, progress)
        .await?;
    Ok(PhaseOutcome::Advanced)
}

async fn playlist_titles(
    handler: &AdminMaintenanceHandler,
    state: &RunState,
) -> JobResult<PhaseOutcome> {
    let cursor = state.cursor_text.clone().unwrap_or_default();
    let rows = sqlx::query_file!(
        "queries/admin_maintenance/playlists_title_scan.sql",
        &cursor,
        handler.scan_batch
    )
    .fetch_all(&handler.pool)
    .await
    .map_err(JobError::retryable)?;

    let Some(tail) = rows.last().map(|row| row.urn.clone()) else {
        handler
            .advance_phase(CATALOG_RENORMALIZE, state, TRACK_META)
            .await?;
        return Ok(PhaseOutcome::Advanced);
    };

    let mut progress = SliceProgress {
        scanned: rows.len() as i64,
        ..SliceProgress::default()
    };
    for row in &rows {
        let fresh = normalize_title(&row.title);
        if fresh == row.title_normalized {
            continue;
        }
        sqlx::query_file!(
            "queries/admin_maintenance/playlist_set_title_norm.sql",
            &row.urn,
            fresh
        )
        .execute(&handler.pool)
        .await
        .map_err(JobError::retryable)?;
        progress.changed += 1;
    }

    handler
        .save_progress(CATALOG_RENORMALIZE, state, None, Some(&tail), progress)
        .await?;
    Ok(PhaseOutcome::Advanced)
}

async fn track_meta(
    handler: &AdminMaintenanceHandler,
    state: &RunState,
) -> JobResult<PhaseOutcome> {
    let cursor = state.cursor_uuid.unwrap_or(Uuid::nil());
    let rows = sqlx::query_file!(
        "queries/admin_maintenance/tracks_meta_escaped_scan.sql",
        cursor,
        handler.scan_batch
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
        let Some(meta) = row.metadata_artist.as_deref() else {
            continue;
        };
        let fresh = unescape_json_unicode(meta);
        if fresh == meta {
            continue;
        }
        sqlx::query_file!(
            "queries/admin_maintenance/track_set_meta.sql",
            row.id,
            fresh
        )
        .execute(&handler.pool)
        .await
        .map_err(JobError::retryable)?;
        progress.changed += 1;
    }

    handler
        .save_progress(CATALOG_RENORMALIZE, state, Some(tail), None, progress)
        .await?;
    Ok(PhaseOutcome::Advanced)
}

pub(super) async fn merge_root(handler: &AdminMaintenanceHandler, id: Uuid) -> JobResult<Uuid> {
    let mut current = id;
    for _ in 0..4 {
        let next: Option<Option<Uuid>> =
            sqlx::query_file_scalar!("queries/admin_maintenance/artist_merged_into.sql", current)
                .fetch_optional(&handler.pool)
                .await
                .map_err(JobError::retryable)?;
        match next {
            Some(Some(parent)) => current = parent,
            _ => break,
        }
    }
    Ok(current)
}

pub(super) async fn repoint_references(
    handler: &AdminMaintenanceHandler,
    from: Uuid,
    to: Uuid,
) -> JobResult {
    if from == to {
        return Ok(());
    }
    let mut transaction = handler.pool.begin().await.map_err(JobError::retryable)?;
    for query in [
        "queries/admin_maintenance/merge_repoint_track_artists_dedup.sql",
        "queries/admin_maintenance/merge_repoint_track_artists.sql",
        "queries/admin_maintenance/merge_repoint_tracks_primary.sql",
        "queries/admin_maintenance/merge_repoint_tracks_cover.sql",
        "queries/admin_maintenance/merge_repoint_albums_primary.sql",
        "queries/admin_maintenance/merge_repoint_album_artists_dedup.sql",
        "queries/admin_maintenance/merge_repoint_album_artists.sql",
    ] {
        run_repoint(&mut transaction, query, from, to).await?;
    }
    transaction.commit().await.map_err(JobError::retryable)?;
    Ok(())
}

async fn run_repoint(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    query: &str,
    from: Uuid,
    to: Uuid,
) -> JobResult {
    let result = match query {
        "queries/admin_maintenance/merge_repoint_track_artists_dedup.sql" => {
            sqlx::query_file!(
                "queries/admin_maintenance/merge_repoint_track_artists_dedup.sql",
                from,
                to
            )
            .execute(&mut **transaction)
            .await
        }
        "queries/admin_maintenance/merge_repoint_track_artists.sql" => {
            sqlx::query_file!(
                "queries/admin_maintenance/merge_repoint_track_artists.sql",
                from,
                to
            )
            .execute(&mut **transaction)
            .await
        }
        "queries/admin_maintenance/merge_repoint_tracks_primary.sql" => {
            sqlx::query_file!(
                "queries/admin_maintenance/merge_repoint_tracks_primary.sql",
                from,
                to
            )
            .execute(&mut **transaction)
            .await
        }
        "queries/admin_maintenance/merge_repoint_tracks_cover.sql" => {
            sqlx::query_file!(
                "queries/admin_maintenance/merge_repoint_tracks_cover.sql",
                from,
                to
            )
            .execute(&mut **transaction)
            .await
        }
        "queries/admin_maintenance/merge_repoint_albums_primary.sql" => {
            sqlx::query_file!(
                "queries/admin_maintenance/merge_repoint_albums_primary.sql",
                from,
                to
            )
            .execute(&mut **transaction)
            .await
        }
        "queries/admin_maintenance/merge_repoint_album_artists_dedup.sql" => {
            sqlx::query_file!(
                "queries/admin_maintenance/merge_repoint_album_artists_dedup.sql",
                from,
                to
            )
            .execute(&mut **transaction)
            .await
        }
        _ => {
            sqlx::query_file!(
                "queries/admin_maintenance/merge_repoint_album_artists.sql",
                from,
                to
            )
            .execute(&mut **transaction)
            .await
        }
    };
    result.map(|_| ()).map_err(JobError::retryable)
}

pub(super) fn is_unique_violation(error: &sqlx::Error) -> bool {
    error
        .as_database_error()
        .and_then(|error| error.code())
        .is_some_and(|code| code == "23505")
}
