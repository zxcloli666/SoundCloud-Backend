use axum::Json;
use axum::extract::{Path, Query, State};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::common::admin::AdminAuth;
use crate::error::{AppError, AppResult};
use crate::state::AppState;

const DRAINABLE: &[&str] = &["equal", "remote_superset", "order_only"];

#[derive(Deserialize)]
pub struct LegacyQuery {
    #[serde(default)]
    pub classification: Option<String>,
    #[serde(default)]
    pub page: Option<i64>,
    #[serde(default)]
    pub limit: Option<i64>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LegacyIntentRow {
    pub archive_id: Uuid,
    pub playlist_urn: String,
    pub source: String,
    pub classification: String,
    pub legacy_user_id: Option<String>,
    pub legacy_desired_revision: Option<i64>,
    pub legacy_synced_revision: Option<i64>,
    pub queue_last_error: Option<String>,
    pub archived_at: chrono::DateTime<chrono::Utc>,
    pub remote_track_count: Option<i32>,
    pub projection_track_count: Option<i32>,
    pub sync_status: Option<String>,
    pub drains_automatically: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClassificationCount {
    pub classification: String,
    pub count: i64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LegacyIntentsPage {
    pub items: Vec<LegacyIntentRow>,
    pub page: i64,
    pub limit: i64,
    pub by_classification: Vec<ClassificationCount>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AbandonResult {
    pub archive_id: Uuid,
    pub playlist_urn: String,
    pub playlist_unblocked: bool,
}

#[tracing::instrument(skip_all)]
pub async fn list_legacy(
    _: AdminAuth,
    State(state): State<AppState>,
    Query(q): Query<LegacyQuery>,
) -> AppResult<Json<LegacyIntentsPage>> {
    let limit = q.limit.unwrap_or(50).clamp(1, 200);
    let page = q.page.unwrap_or(1).max(1);
    let offset = (page - 1) * limit;
    let classification = q.classification.filter(|value| !value.is_empty());

    let counts = sqlx::query_file!("queries/admin/playlists/legacy_counts.sql")
        .fetch_all(&state.pg)
        .await?;
    let rows = sqlx::query_file!(
        "queries/admin/playlists/legacy_list.sql",
        classification.as_deref(),
        limit,
        offset
    )
    .fetch_all(&state.pg)
    .await?;

    Ok(Json(LegacyIntentsPage {
        items: rows
            .into_iter()
            .map(|row| LegacyIntentRow {
                archive_id: row.archive_id,
                playlist_urn: row.playlist_urn,
                source: row.source,
                drains_automatically: DRAINABLE.contains(&row.classification.as_str()),
                classification: row.classification,
                legacy_user_id: row.legacy_user_id,
                legacy_desired_revision: row.legacy_desired_revision,
                legacy_synced_revision: row.legacy_synced_revision,
                queue_last_error: row.queue_last_error,
                archived_at: row.archived_at,
                remote_track_count: row.remote_track_count,
                projection_track_count: row.projection_track_count,
                sync_status: row.sync_status,
            })
            .collect(),
        page,
        limit,
        by_classification: counts
            .into_iter()
            .map(|row| ClassificationCount {
                classification: row.classification,
                count: row.count,
            })
            .collect(),
    }))
}

#[tracing::instrument(skip_all)]
pub async fn abandon_legacy(
    _: AdminAuth,
    State(state): State<AppState>,
    Path(archive_id): Path<Uuid>,
) -> AppResult<Json<AbandonResult>> {
    let mut transaction = state.pg.begin().await?;
    let playlist_urn =
        sqlx::query_file_scalar!("queries/admin/playlists/legacy_abandon.sql", archive_id)
            .fetch_optional(&mut *transaction)
            .await?
            .ok_or_else(|| {
                AppError::not_found("legacy playlist intent not found or already resolved")
            })?;
    let unblocked = sqlx::query_file!("queries/admin/playlists/legacy_wake.sql", &playlist_urn)
        .execute(&mut *transaction)
        .await?
        .rows_affected()
        > 0;
    transaction.commit().await?;

    Ok(Json(AbandonResult {
        archive_id,
        playlist_urn,
        playlist_unblocked: unblocked,
    }))
}
