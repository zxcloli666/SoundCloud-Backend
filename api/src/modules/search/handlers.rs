use axum::extract::{Query, State};
use axum::routing::get;
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::Value;

use crate::cache::ListPageResult;
use crate::common::pagination::PaginationQuery;
use crate::common::session::SessionCtx;
use crate::error::AppResult;
use crate::modules::search::lyrics::LyricsSearchResponse;
use crate::modules::search::vibe::VibeResponse;
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/search/db/tracks", get(tracks))
        .route("/search/db/playlists", get(playlists))
        .route("/search/db/users", get(users))
        .route("/search/db/artists", get(artists))
        .route("/search/db/albums", get(albums))
        .route("/search/vibe", get(vibe))
        .route("/search/lyrics", get(lyrics))
}

#[derive(Debug, Deserialize)]
struct VibeQuery {
    #[serde(default)]
    q: Option<String>,
    #[serde(default)]
    limit: Option<String>,
}

async fn vibe(
    State(st): State<AppState>,
    _ctx: SessionCtx,
    Query(q): Query<VibeQuery>,
) -> AppResult<Json<VibeResponse>> {
    let limit = q.limit.as_deref().and_then(|s| s.parse::<usize>().ok());
    Ok(Json(st.vibe.vibe(&q.q.unwrap_or_default(), limit).await?))
}

#[derive(Debug, Deserialize)]
struct LyricsQuery {
    #[serde(default)]
    q: Option<String>,
    #[serde(default)]
    page: Option<String>,
    #[serde(default)]
    limit: Option<String>,
}

async fn lyrics(
    State(st): State<AppState>,
    _ctx: SessionCtx,
    Query(q): Query<LyricsQuery>,
) -> AppResult<Json<LyricsSearchResponse>> {
    let page = q.page.as_deref().and_then(|s| s.parse::<i64>().ok());
    let limit = q.limit.as_deref().and_then(|s| s.parse::<i64>().ok());
    Ok(Json(
        st.search
            .lyrics(&q.q.unwrap_or_default(), page, limit)
            .await?,
    ))
}

#[derive(Debug, Clone, Deserialize)]
struct CatalogQuery {
    #[serde(default)]
    q: Option<String>,
    #[serde(default)]
    user_urn: Option<String>,
}

async fn tracks(
    State(st): State<AppState>,
    _ctx: SessionCtx,
    Query(p): Query<PaginationQuery>,
    Query(q): Query<CatalogQuery>,
) -> AppResult<Json<ListPageResult<Value>>> {
    let (page, limit) = p.resolved();
    let query = q.q.unwrap_or_default();
    Ok(Json(
        st.search
            .tracks(&query, q.user_urn.as_deref(), page, limit)
            .await?,
    ))
}

async fn playlists(
    State(st): State<AppState>,
    _ctx: SessionCtx,
    Query(p): Query<PaginationQuery>,
    Query(q): Query<CatalogQuery>,
) -> AppResult<Json<ListPageResult<Value>>> {
    let (page, limit) = p.resolved();
    let query = q.q.unwrap_or_default();
    Ok(Json(
        st.search
            .playlists(&query, q.user_urn.as_deref(), page, limit)
            .await?,
    ))
}

async fn users(
    State(st): State<AppState>,
    _ctx: SessionCtx,
    Query(p): Query<PaginationQuery>,
    Query(q): Query<CatalogQuery>,
) -> AppResult<Json<ListPageResult<Value>>> {
    let (page, limit) = p.resolved();
    Ok(Json(
        st.search
            .users(&q.q.unwrap_or_default(), page, limit)
            .await?,
    ))
}

async fn artists(
    State(st): State<AppState>,
    _ctx: SessionCtx,
    Query(p): Query<PaginationQuery>,
    Query(q): Query<CatalogQuery>,
) -> AppResult<Json<ListPageResult<Value>>> {
    let (page, limit) = p.resolved();
    Ok(Json(
        st.search
            .artists(&q.q.unwrap_or_default(), page, limit)
            .await?,
    ))
}

async fn albums(
    State(st): State<AppState>,
    _ctx: SessionCtx,
    Query(p): Query<PaginationQuery>,
    Query(q): Query<CatalogQuery>,
) -> AppResult<Json<ListPageResult<Value>>> {
    let (page, limit) = p.resolved();
    Ok(Json(
        st.search
            .albums(&q.q.unwrap_or_default(), page, limit)
            .await?,
    ))
}
