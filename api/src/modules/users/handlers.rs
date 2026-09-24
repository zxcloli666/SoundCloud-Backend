use axum::extract::{Path, Query, State};
use axum::response::Response;
use axum::routing::get;
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::Value;

use crate::cache::ListPageResult;
use crate::common::cache_helper::cached_or_fetch;
use crate::common::pagination::PaginationQuery;
use crate::common::sc_ids::extract_sc_id;
use crate::common::session::SessionCtx;
use crate::error::AppResult;
use crate::modules::cold_refresh::collection::CollectionPage;
use crate::modules::enrich::dto as enrich_dto;
use crate::modules::me::service::premium_response;
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/users", get(search))
        .route("/users/{user_urn}", get(get_by_id))
        .route("/users/{user_urn}/followers", get(get_followers))
        .route("/users/{user_urn}/followings", get(get_followings))
        .route(
            "/users/{user_urn}/followings/{following_urn}",
            get(get_is_following),
        )
        .route("/users/{user_urn}/tracks", get(get_tracks))
        .route("/users/{user_urn}/playlists", get(get_playlists))
        .route("/users/{user_urn}/likes/tracks", get(get_liked_tracks))
        .route(
            "/users/{user_urn}/likes/playlists",
            get(get_liked_playlists),
        )
        .route("/users/{user_urn}/subscription", get(get_subscription))
        .route("/users/{user_urn}/web-profiles", get(get_web_profiles))
}

#[derive(Debug, Clone, Deserialize)]
struct SearchQuery {
    #[serde(default)]
    q: Option<String>,
    #[serde(default)]
    ids: Option<String>,
}

async fn search(
    State(st): State<AppState>,
    _ctx: SessionCtx,
    Query(p): Query<PaginationQuery>,
    Query(q): Query<SearchQuery>,
) -> AppResult<Json<ListPageResult<Value>>> {
    let (page, limit) = p.resolved();
    Ok(Json(
        st.search
            .users(
                q.q.as_deref().unwrap_or_default(),
                q.ids.as_deref(),
                page,
                limit,
            )
            .await?,
    ))
}

async fn get_by_id(
    State(st): State<AppState>,
    _ctx: SessionCtx,
    Path(user_urn): Path<String>,
) -> AppResult<Json<Value>> {
    Ok(Json(st.users.get_by_id(&user_urn).await?))
}

async fn get_followers(
    State(st): State<AppState>,
    _ctx: SessionCtx,
    Path(user_urn): Path<String>,
    Query(p): Query<PaginationQuery>,
) -> AppResult<Json<CollectionPage>> {
    let (page, limit) = p.resolved();
    Ok(Json(st.users.get_followers(&user_urn, page, limit).await?))
}

async fn get_followings(
    State(st): State<AppState>,
    ctx: SessionCtx,
    Path(user_urn): Path<String>,
    Query(p): Query<PaginationQuery>,
) -> AppResult<Json<CollectionPage>> {
    let (page, limit) = p.resolved();
    let target = extract_sc_id(&user_urn);
    Ok(Json(
        st.users
            .get_followings(&ctx.sc_user_id, target, page, limit)
            .await?,
    ))
}

async fn get_is_following(
    State(st): State<AppState>,
    ctx: SessionCtx,
    Path((user_urn, following_urn)): Path<(String, String)>,
) -> AppResult<Json<bool>> {
    Ok(Json(
        st.users
            .get_is_following(&ctx.sc_user_id, &user_urn, &following_urn)
            .await?,
    ))
}

async fn get_tracks(
    State(st): State<AppState>,
    ctx: SessionCtx,
    Path(user_urn): Path<String>,
    Query(p): Query<PaginationQuery>,
) -> AppResult<Json<CollectionPage>> {
    let (page, limit) = p.resolved();
    let target = extract_sc_id(&user_urn);
    let mut result = st
        .users
        .get_owned_tracks(&ctx.sc_user_id, target, page, limit)
        .await?;
    enrich_dto::apply_to_tracks(&st.pg, &mut result.collection).await?;
    Ok(Json(result))
}

async fn get_playlists(
    State(st): State<AppState>,
    ctx: SessionCtx,
    Path(user_urn): Path<String>,
    Query(p): Query<PaginationQuery>,
) -> AppResult<Json<CollectionPage>> {
    let (page, limit) = p.resolved();
    let target = extract_sc_id(&user_urn);
    Ok(Json(
        st.users
            .get_owned_playlists(&ctx.sc_user_id, target, page, limit)
            .await?,
    ))
}

async fn get_liked_tracks(
    State(st): State<AppState>,
    ctx: SessionCtx,
    Path(user_urn): Path<String>,
    Query(p): Query<PaginationQuery>,
) -> AppResult<Json<CollectionPage>> {
    let (page, limit) = p.resolved();
    let target = extract_sc_id(&user_urn);
    let mut result = st
        .users
        .get_liked_tracks(&ctx.sc_user_id, target, page, limit)
        .await?;
    enrich_dto::apply_to_tracks(&st.pg, &mut result.collection).await?;
    Ok(Json(result))
}

async fn get_liked_playlists(
    State(st): State<AppState>,
    ctx: SessionCtx,
    Path(user_urn): Path<String>,
    Query(p): Query<PaginationQuery>,
) -> AppResult<Json<CollectionPage>> {
    let (page, limit) = p.resolved();
    let target = extract_sc_id(&user_urn);
    Ok(Json(
        st.users
            .get_liked_playlists(&ctx.sc_user_id, target, page, limit)
            .await?,
    ))
}

async fn get_subscription(
    State(st): State<AppState>,
    _ctx: SessionCtx,
    Path(user_urn): Path<String>,
) -> AppResult<Response> {
    let url = format!("/users/{user_urn}/subscription");
    cached_or_fetch(
        &st,
        crate::common::cache_helper::CacheOpts {
            method: "GET",
            url: &url,
            ttl_sec: 300,
            cache_key: None,
        },
        || async {
            let premium = st.subscriptions.is_premium(&user_urn).await?;
            Ok(premium_response(premium))
        },
    )
    .await
}

async fn get_web_profiles(
    State(st): State<AppState>,
    _ctx: SessionCtx,
    Path(user_urn): Path<String>,
) -> AppResult<Json<Value>> {
    Ok(Json(st.users.get_web_profiles(&user_urn).await?))
}
