use axum::extract::State;
use axum::routing::get;
use axum::{Json, Router};
use backend_contracts::{EmptyPayload, JobKind};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::background_jobs::BackgroundJob;
use crate::common::admin::AdminAuth;
use crate::error::{AppError, AppResult};
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/admin/discover/promoted",
            get(admin_promoted_list).post(admin_promoted_create),
        )
        .route(
            "/admin/discover/promoted/{id}",
            axum::routing::patch(admin_promoted_update).delete(admin_promoted_delete),
        )
        .route(
            "/admin/discover/settings",
            get(admin_settings_get).patch(admin_settings_update),
        )
        .route(
            "/admin/discover/refresh",
            axum::routing::post(admin_refresh),
        )
}

#[tracing::instrument(skip_all)]
pub async fn admin_refresh(
    _: AdminAuth,
    State(st): State<AppState>,
) -> AppResult<Json<serde_json::Value>> {
    let job = BackgroundJob::coalescing(JobKind::DiscoverAggregates, "schedule", EmptyPayload {})?
        .with_priority(10)
        .with_max_attempts(4)?;
    let job_id = st.background_jobs.enqueue(&job).await?;
    Ok(Json(serde_json::json!({ "ok": true, "jobId": job_id })))
}

#[derive(Debug, Serialize, sqlx::FromRow)]
struct AdminPromotedRow {
    id: Uuid,
    entity_type: String,
    entity_id: Uuid,
    position: i32,
    active: bool,
    note: Option<String>,
    created_at: chrono::DateTime<chrono::Utc>,
    updated_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Serialize, sqlx::FromRow)]
struct AdminPromotedListRow {
    id: Uuid,
    entity_type: String,
    entity_id: Uuid,
    position: i32,
    active: bool,
    note: Option<String>,
    created_at: chrono::DateTime<chrono::Utc>,
    updated_at: chrono::DateTime<chrono::Utc>,
    name: Option<String>,
    image_url: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AdminPromotedCreate {
    entity_type: String,
    entity_id: Uuid,
    #[serde(default)]
    position: Option<i32>,
    #[serde(default)]
    active: Option<bool>,
    #[serde(default)]
    note: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AdminPromotedUpdate {
    #[serde(default)]
    position: Option<i32>,
    #[serde(default)]
    active: Option<bool>,
    #[serde(default)]
    note: Option<Option<String>>,
}

async fn admin_promoted_list(
    _: AdminAuth,
    State(st): State<AppState>,
) -> AppResult<Json<Vec<AdminPromotedListRow>>> {
    let rows = sqlx::query_file_as!(
        AdminPromotedListRow,
        "queries/discover/handlers/admin_promoted_list.sql"
    )
    .fetch_all(&st.pg)
    .await?;
    Ok(Json(rows))
}

async fn admin_promoted_create(
    _: AdminAuth,
    State(st): State<AppState>,
    Json(body): Json<AdminPromotedCreate>,
) -> AppResult<Json<AdminPromotedRow>> {
    if body.entity_type != "artist" && body.entity_type != "album" {
        return Err(AppError::bad_request(
            "entity_type must be 'artist' or 'album'",
        ));
    }
    let row: AdminPromotedRow = sqlx::query_as(
        r#"INSERT INTO discover_promoted (entity_type, entity_id, position, active, note)
           VALUES ($1, $2, COALESCE($3, 0), COALESCE($4, TRUE), $5)
           ON CONFLICT (entity_type, entity_id) DO UPDATE SET
               position = COALESCE($3, discover_promoted.position),
               active   = COALESCE($4, discover_promoted.active),
               note     = COALESCE($5, discover_promoted.note),
               updated_at = NOW()
           RETURNING id, entity_type, entity_id, position, active, note, created_at, updated_at"#,
    )
    .bind(&body.entity_type)
    .bind(body.entity_id)
    .bind(body.position)
    .bind(body.active)
    .bind(body.note)
    .fetch_one(&st.pg)
    .await?;
    Ok(Json(row))
}

async fn admin_promoted_update(
    _: AdminAuth,
    State(st): State<AppState>,
    axum::extract::Path(id): axum::extract::Path<Uuid>,
    Json(body): Json<AdminPromotedUpdate>,
) -> AppResult<Json<AdminPromotedRow>> {
    let note_set = body.note.is_some();
    let note_value = body.note.unwrap_or(None);
    let row: Option<AdminPromotedRow> = sqlx::query_as(
        r#"UPDATE discover_promoted SET
               position   = COALESCE($2, position),
               active     = COALESCE($3, active),
               note       = CASE WHEN $4::bool THEN $5 ELSE note END,
               updated_at = NOW()
           WHERE id = $1
           RETURNING id, entity_type, entity_id, position, active, note, created_at, updated_at"#,
    )
    .bind(id)
    .bind(body.position)
    .bind(body.active)
    .bind(note_set)
    .bind(note_value)
    .fetch_optional(&st.pg)
    .await?;
    row.map(Json)
        .ok_or_else(|| AppError::not_found("promoted not found"))
}

async fn admin_promoted_delete(
    _: AdminAuth,
    State(st): State<AppState>,
    axum::extract::Path(id): axum::extract::Path<Uuid>,
) -> AppResult<Json<serde_json::Value>> {
    let n = sqlx::query_file!("queries/discover/handlers/promoted_delete.sql", id)
        .execute(&st.pg)
        .await?
        .rows_affected();
    Ok(Json(serde_json::json!({ "deleted": n })))
}

#[derive(Debug, Serialize, sqlx::FromRow)]
struct AdminSettingsRow {
    show_star: bool,
    star_strategy: String,
    star_limit: i32,
    updated_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Deserialize)]
struct AdminSettingsUpdate {
    #[serde(default)]
    show_star: Option<bool>,
    #[serde(default)]
    star_strategy: Option<String>,
    #[serde(default)]
    star_limit: Option<i32>,
}

async fn admin_settings_get(
    _: AdminAuth,
    State(st): State<AppState>,
) -> AppResult<Json<AdminSettingsRow>> {
    let row = sqlx::query_file_as!(
        AdminSettingsRow,
        "queries/discover/handlers/admin_settings_get.sql"
    )
    .fetch_one(&st.pg)
    .await?;
    Ok(Json(row))
}

async fn admin_settings_update(
    _: AdminAuth,
    State(st): State<AppState>,
    Json(body): Json<AdminSettingsUpdate>,
) -> AppResult<Json<AdminSettingsRow>> {
    if let Some(s) = body.star_strategy.as_deref()
        && s != "popular"
        && s != "random"
    {
        return Err(AppError::bad_request(
            "star_strategy must be 'popular' or 'random'",
        ));
    }
    let row: AdminSettingsRow = sqlx::query_as(
        r#"UPDATE discover_settings SET
               show_star     = COALESCE($1, show_star),
               star_strategy = COALESCE($2, star_strategy),
               star_limit    = COALESCE($3, star_limit),
               updated_at    = NOW()
           WHERE id = 1
           RETURNING show_star, star_strategy, star_limit, updated_at"#,
    )
    .bind(body.show_star)
    .bind(body.star_strategy)
    .bind(body.star_limit)
    .fetch_one(&st.pg)
    .await?;
    Ok(Json(row))
}
