use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::Response;
use axum::routing::get;
use axum::{Json, Router};
use serde::Deserialize;

use crate::cache::cache_service::CacheScope;
use crate::common::response::json_response;
use crate::common::session::SessionCtx;
use crate::error::{AppError, AppResult};
use crate::modules::history::service::{HistoryPage, RecordHistoryDto};
use crate::modules::history::stats::{StatsPeriod, clamp_utc_offset};
use crate::state::AppState;

const STATS_CACHE_KEY: &str = "history-stats";
const STATS_TTL_SEC: u64 = 300;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/history", get(find_all).post(record).delete(clear))
        .route("/history/stats", get(stats))
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StatsQuery {
    #[serde(default)]
    period: StatsPeriod,
    #[serde(default)]
    utc_offset: i32,
}

#[derive(Debug, Clone, Deserialize)]
struct PageQuery {
    #[serde(default)]
    limit: Option<String>,
    #[serde(default)]
    offset: Option<String>,
}

async fn record(
    State(st): State<AppState>,
    ctx: SessionCtx,
    Json(body): Json<RecordHistoryDto>,
) -> AppResult<StatusCode> {
    st.history.record(&ctx.sc_user_id, &body).await?;
    forget_stats(&st, &ctx.sc_user_id).await;
    Ok(StatusCode::OK)
}

async fn find_all(
    State(st): State<AppState>,
    ctx: SessionCtx,
    Query(q): Query<PageQuery>,
) -> AppResult<Json<HistoryPage>> {
    let limit = q
        .limit
        .as_deref()
        .and_then(|s| s.parse::<i64>().ok())
        .unwrap_or(50)
        .min(200);
    let offset = q
        .offset
        .as_deref()
        .and_then(|s| s.parse::<i64>().ok())
        .unwrap_or(0);
    Ok(Json(
        st.history.find_all(&ctx.sc_user_id, limit, offset).await?,
    ))
}

async fn clear(State(st): State<AppState>, ctx: SessionCtx) -> AppResult<StatusCode> {
    st.history.clear(&ctx.sc_user_id).await?;
    forget_stats(&st, &ctx.sc_user_id).await;
    Ok(StatusCode::OK)
}

async fn stats(
    State(st): State<AppState>,
    ctx: SessionCtx,
    Query(q): Query<StatsQuery>,
) -> AppResult<Response> {
    let utc_offset = clamp_utc_offset(q.utc_offset);
    let url = format!(
        "/history/stats?period={}&utcOffset={utc_offset}",
        q.period.as_str()
    );
    let key = st
        .cache
        .build_key("GET", &url, CacheScope::User, Some(&ctx.sc_user_id));
    if let Ok(Some(raw)) = st.cache.get_raw(&key).await {
        return Ok(json_response(StatusCode::OK, raw));
    }
    let now = chrono::Utc::now().naive_utc();
    let stats = st
        .history
        .stats(&ctx.sc_user_id, q.period, utc_offset, now)
        .await?;
    let payload = serde_json::to_string(&stats)
        .map_err(|error| AppError::internal(format!("json encode: {error}")))?;
    let _ = st
        .cache
        .set_raw(
            &key,
            &payload,
            STATS_TTL_SEC,
            Some(STATS_CACHE_KEY),
            CacheScope::User,
            Some(&ctx.sc_user_id),
        )
        .await;
    Ok(json_response(StatusCode::OK, payload))
}

async fn forget_stats(st: &AppState, sc_user_id: &str) {
    let _ = st
        .cache
        .clear_by_cache_keys(&[STATS_CACHE_KEY.to_owned()], Some(sc_user_id))
        .await;
}
