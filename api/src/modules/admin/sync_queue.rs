use axum::Json;
use axum::extract::{Query, State};
use backend_contracts::{JobKind, SYNC_QUEUE_MAX_RETRIES, SyncQueueFlushPayload};
use serde::{Deserialize, Serialize};

use crate::background_jobs::BackgroundJob;
use crate::common::admin::AdminAuth;
use crate::error::{AppError, AppResult};
use crate::state::AppState;

#[derive(Serialize)]
pub struct ActionCount {
    pub action_type: String,
    pub count: i64,
}

#[derive(Deserialize)]
pub struct ListQuery {
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub limit: Option<i64>,
    #[serde(default)]
    pub offset: Option<i64>,
}

#[derive(Serialize)]
pub struct SyncQueueItem {
    pub id: uuid::Uuid,
    pub user_id: String,
    pub action_type: String,
    pub target_urn: Option<String>,
    pub retry_count: i32,
    pub last_error: Option<String>,
    pub next_run_at: Option<chrono::DateTime<chrono::Utc>>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub dead: bool,
    pub failed_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[tracing::instrument(skip_all)]
pub async fn list_items(
    _: AdminAuth,
    State(state): State<AppState>,
    Query(q): Query<ListQuery>,
) -> AppResult<Json<Vec<SyncQueueItem>>> {
    let status = q.status.unwrap_or_else(|| "all".into());
    if !matches!(status.as_str(), "all" | "pending" | "retrying" | "dead") {
        return Err(AppError::bad_request(
            "status must be all|pending|retrying|dead",
        ));
    }
    let limit = q.limit.unwrap_or(50).clamp(1, 200);
    let offset = q.offset.unwrap_or(0).max(0);
    let rows = sqlx::query_file_as!(
        SyncQueueItem,
        "queries/admin/sync_queue/list_items.sql",
        status,
        limit,
        offset
    )
    .fetch_all(&state.pg)
    .await?;
    Ok(Json(rows))
}

#[derive(Serialize)]
pub struct SyncQueueStats {
    pub pending: i64,
    pub failed: i64,
    pub dead: i64,
    pub oldest_pending_at: Option<chrono::DateTime<chrono::Utc>>,
    pub by_action: Vec<ActionCount>,
    pub recent_errors: Vec<String>,
}

#[tracing::instrument(skip_all)]
pub async fn get_stats(
    _: AdminAuth,
    State(state): State<AppState>,
) -> AppResult<Json<SyncQueueStats>> {
    let counts = sqlx::query_file!("queries/admin/sync_queue/stats_counts.sql")
        .fetch_one(&state.pg)
        .await?;
    let rows = sqlx::query_file!("queries/admin/sync_queue/stats_by_action.sql")
        .fetch_all(&state.pg)
        .await?;
    let by_action = rows
        .into_iter()
        .map(|r| ActionCount {
            action_type: r.action_type,
            count: r.count,
        })
        .collect();
    let recent_errors: Vec<String> =
        sqlx::query_file_scalar!("queries/admin/sync_queue/recent_errors.sql")
            .fetch_all(&state.pg)
            .await?;

    Ok(Json(SyncQueueStats {
        pending: counts.pending,
        failed: counts.failed,
        dead: counts.dead,
        oldest_pending_at: counts.oldest_pending_at,
        by_action,
        recent_errors,
    }))
}

#[derive(Serialize)]
pub struct FlushResponse {
    pub job_id: uuid::Uuid,
}

#[tracing::instrument(skip_all)]
pub async fn flush(_: AdminAuth, State(state): State<AppState>) -> AppResult<Json<FlushResponse>> {
    let job = BackgroundJob::coalescing(
        JobKind::SyncQueueFlush,
        "admin",
        SyncQueueFlushPayload { force: true },
    )?;
    let job_id = state.background_jobs.enqueue(&job).await?;
    Ok(Json(FlushResponse { job_id }))
}

#[derive(Deserialize)]
pub struct PurgeQuery {
    #[serde(default = "default_min_retries")]
    pub min_retries: i32,
}

fn default_min_retries() -> i32 {
    SYNC_QUEUE_MAX_RETRIES - 1
}

#[derive(Serialize)]
pub struct PurgeResponse {
    pub purged: u64,
    pub min_retries: i32,
}

#[tracing::instrument(skip_all)]
pub async fn purge(
    _: AdminAuth,
    State(state): State<AppState>,
    Query(q): Query<PurgeQuery>,
) -> AppResult<Json<PurgeResponse>> {
    if q.min_retries < 1 {
        return Err(AppError::bad_request("min_retries must be >= 1"));
    }
    let res = sqlx::query_file!("queries/admin/sync_queue/purge.sql", q.min_retries)
        .execute(&state.pg)
        .await?;
    Ok(Json(PurgeResponse {
        purged: res.rows_affected(),
        min_retries: q.min_retries,
    }))
}
