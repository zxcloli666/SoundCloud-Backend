use axum::extract::{Query, State};
use axum::http::HeaderMap;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use serde::Deserialize;

use super::query::{LiveKind, plain_tracks};
use crate::common::pagination::PaginationQuery;
use crate::common::session::SessionCtx;
use crate::error::AppResult;
use crate::modules::search::query::TrackSearchQuery;
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/users", get(users))
        .route("/search/db/tracks", get(db_tracks))
}

#[derive(Debug, Deserialize)]
struct UserSearchQuery {
    #[serde(default)]
    q: Option<String>,
    #[serde(default)]
    ids: Option<String>,
}

async fn users(
    State(st): State<AppState>,
    ctx: SessionCtx,
    headers: HeaderMap,
    Query(p): Query<PaginationQuery>,
    Query(q): Query<UserSearchQuery>,
) -> AppResult<Response> {
    let (page, limit) = p.resolved();
    let text = q.q.as_deref().unwrap_or_default();
    let plain = q.q.as_deref().filter(|_| q.ids.is_none());
    let Some(request) =
        st.live_search
            .plan(LiveKind::Users, plain, &headers, &p, false, &ctx.sc_user_id)
    else {
        let result = st.search.users(text, q.ids.as_deref(), page, limit).await?;
        return Ok(Json(result).into_response());
    };
    let live = st
        .live_search
        .page(&request, |local_page| {
            st.search.users(text, None, local_page, request.limit)
        })
        .await?;
    Ok(live.into_response())
}

async fn db_tracks(
    State(st): State<AppState>,
    ctx: SessionCtx,
    headers: HeaderMap,
    Query(p): Query<PaginationQuery>,
    Query(q): Query<TrackSearchQuery>,
) -> AppResult<Response> {
    let (page, limit) = p.resolved();
    let result = st.search.tracks(&q, page, limit).await?;
    let rescue = st
        .live_search
        .rescue_plan(plain_tracks(&q), &headers, &p, &ctx.sc_user_id);
    let Some(request) = rescue.filter(|_| result.collection.is_empty()) else {
        return Ok(Json(result).into_response());
    };
    let live = st
        .live_search
        .page(&request, |_| {
            let empty = result.clone();
            async move { Ok(empty) }
        })
        .await?;
    Ok(live.into_response())
}
