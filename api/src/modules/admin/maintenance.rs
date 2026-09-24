use axum::Json;
use axum::extract::State;
use backend_contracts::{AdminMaintenancePayload, JobKind};
use chrono::{DateTime, Utc};
use serde::Serialize;
use uuid::Uuid;

use crate::background_jobs::BackgroundJob;
use crate::common::admin::AdminAuth;
use crate::error::AppResult;
use crate::state::AppState;

const CATALOG_RENORMALIZE: &str = "catalog_renormalize";
const MUSICBRAINZ_NAMES: &str = "musicbrainz_names";
const MAINTENANCE_PRIORITY: i16 = 20;
const MAINTENANCE_MAX_ATTEMPTS: i16 = 8;

#[derive(Serialize)]
pub struct MaintenanceAccepted {
    pub kind: &'static str,
    pub run_id: Uuid,
    pub started: bool,
}

#[derive(Serialize, sqlx::FromRow)]
pub struct MaintenanceRun {
    pub kind: String,
    pub run_id: Uuid,
    pub status: String,
    pub phase: String,
    pub scanned: i64,
    pub changed: i64,
    pub merged: i64,
    pub skipped: i64,
    pub started_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
}

#[tracing::instrument(skip_all)]
pub async fn renormalize(
    _: AdminAuth,
    State(state): State<AppState>,
) -> AppResult<Json<MaintenanceAccepted>> {
    enqueue(
        &state,
        CATALOG_RENORMALIZE,
        JobKind::AdminCatalogRenormalize,
    )
    .await
}

#[tracing::instrument(skip_all)]
pub async fn mb_artist_names(
    _: AdminAuth,
    State(state): State<AppState>,
) -> AppResult<Json<MaintenanceAccepted>> {
    enqueue(&state, MUSICBRAINZ_NAMES, JobKind::AdminMusicBrainzNames).await
}

#[tracing::instrument(skip_all)]
pub async fn status(
    _: AdminAuth,
    State(state): State<AppState>,
) -> AppResult<Json<Vec<MaintenanceRun>>> {
    let runs = sqlx::query_file_as!(MaintenanceRun, "queries/admin/maintenance_runs/status.sql")
        .fetch_all(&state.pg)
        .await?;
    Ok(Json(runs))
}

async fn enqueue(
    state: &AppState,
    kind: &'static str,
    job_kind: JobKind,
) -> AppResult<Json<MaintenanceAccepted>> {
    let active: Option<Uuid> =
        sqlx::query_file_scalar!("queries/admin/maintenance_runs/active_run.sql", kind)
            .fetch_optional(&state.pg)
            .await?;
    if let Some(run_id) = active {
        return Ok(Json(MaintenanceAccepted {
            kind,
            run_id,
            started: false,
        }));
    }

    let run_id = Uuid::now_v7();
    let job = BackgroundJob::coalescing(job_kind, kind, AdminMaintenancePayload { run_id })?
        .with_priority(MAINTENANCE_PRIORITY)
        .with_max_attempts(MAINTENANCE_MAX_ATTEMPTS)?
        .if_absent();
    state.background_jobs.enqueue(&job).await?;
    Ok(Json(MaintenanceAccepted {
        kind,
        run_id,
        started: true,
    }))
}
