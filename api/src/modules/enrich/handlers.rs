use axum::extract::{Path, State};
use axum::response::Response;
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use backend_contracts::admin_catalog::{AdminCommand, EnrichRetry, ScAccountUpsert};
use backend_contracts::{CrawlArtistPayload, JobKind};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::background_jobs::BackgroundJob;
use crate::common::admin::AdminAuth;
use crate::error::AppResult;
use crate::modules::admin::forward::{RequestKey, forward};
use crate::modules::enrich::stats::{self, EnrichStats};
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/admin/enrich/stats", get(get_stats))
        .route("/admin/enrich/retry", post(post_retry))
        .route(
            "/admin/artists/{artist_id}/sc-accounts",
            post(upsert_account),
        )
        .route(
            "/admin/artists/{artist_id}/sc-accounts/{sc_user_id}",
            delete(delete_account),
        )
        .route(
            "/admin/artists/{src_id}/merge-into/{dst_id}",
            post(merge_artists),
        )
        .route("/admin/artists/{artist_id}/retry-crawl", post(retry_crawl))
        .route("/admin/artists/{artist_id}/run-crawl", post(run_crawl_now))
}

async fn retry_crawl(
    _: AdminAuth,
    State(st): State<AppState>,
    key: RequestKey,
    Path(artist_id): Path<Uuid>,
) -> AppResult<Response> {
    forward(&st, key, AdminCommand::ArtistRetryCrawl { artist_id }).await
}

async fn run_crawl_now(
    _: AdminAuth,
    State(st): State<AppState>,
    Path(artist_id): Path<Uuid>,
) -> AppResult<Json<Value>> {
    let job = BackgroundJob::coalescing(
        JobKind::CrawlArtist,
        artist_id.to_string(),
        CrawlArtistPayload { artist_id },
    )?
    .with_priority(-5);
    let job_id = st.background_jobs.enqueue(&job).await?;
    Ok(Json(json!({ "ok": true, "job_id": job_id })))
}

async fn get_stats(_: AdminAuth, State(st): State<AppState>) -> AppResult<Json<EnrichStats>> {
    Ok(Json(stats::stats(&st.pg).await?))
}

async fn post_retry(
    _: AdminAuth,
    State(st): State<AppState>,
    key: RequestKey,
    Json(body): Json<EnrichRetry>,
) -> AppResult<Response> {
    forward(&st, key, AdminCommand::EnrichRetry(body)).await
}

async fn upsert_account(
    _: AdminAuth,
    State(st): State<AppState>,
    key: RequestKey,
    Path(artist_id): Path<Uuid>,
    Json(body): Json<ScAccountUpsert>,
) -> AppResult<Response> {
    forward(&st, key, AdminCommand::ScAccountUpsert { artist_id, body }).await
}

async fn delete_account(
    _: AdminAuth,
    State(st): State<AppState>,
    key: RequestKey,
    Path((artist_id, sc_user_id)): Path<(Uuid, String)>,
) -> AppResult<Response> {
    let command = AdminCommand::ScAccountDelete {
        artist_id,
        sc_user_id,
    };
    forward(&st, key, command).await
}

async fn merge_artists(
    _: AdminAuth,
    State(st): State<AppState>,
    key: RequestKey,
    Path((src_id, dst_id)): Path<(Uuid, Uuid)>,
) -> AppResult<Response> {
    forward(&st, key, AdminCommand::ArtistMerge { src_id, dst_id }).await
}
