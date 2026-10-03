use axum::extract::{Query, State};
use axum::http::HeaderMap;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use serde::Deserialize;

use super::match_search;
use super::matching::{MatchReply, Wanted};
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
        .route("/search/match", get(search_match))
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
    let found = st.search.track_page(&q, page, limit).await?;
    let rescue = st
        .live_search
        .rescue_plan(plain_tracks(&q), &headers, &p, &ctx.sc_user_id);
    let Some(request) = rescue.filter(|_| found.page.collection.is_empty()) else {
        return Ok(Json(found).into_response());
    };
    let mut live = st
        .live_search
        .page(&request, |_| {
            let empty = found.page.clone();
            async move { Ok(empty) }
        })
        .await?;
    live.weak = found.weak.unwrap_or(live.weak);
    Ok(live.into_response())
}

#[derive(Debug, Deserialize)]
struct MatchQuery {
    #[serde(default)]
    artist: Option<String>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    duration_ms: Option<String>,
}

async fn search_match(
    State(st): State<AppState>,
    ctx: SessionCtx,
    Query(q): Query<MatchQuery>,
) -> AppResult<MatchReply> {
    let wanted = Wanted::parse(
        q.artist.as_deref(),
        q.title.as_deref(),
        q.duration_ms.as_deref(),
    )?;
    match_search::find(
        &st.search,
        &st.live_search,
        &st.pg,
        &wanted,
        &ctx.sc_user_id,
    )
    .await
}
