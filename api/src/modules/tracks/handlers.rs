use std::time::Duration;

use axum::extract::{Path, Query, State};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::{get, put};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::Value;

use crate::cache::ListPageResult;
use crate::common::pagination::PaginationQuery;
use crate::common::session::SessionCtx;
use crate::error::{AppError, AppResult};
use crate::modules::cold_refresh::collection::CollectionPage;
use crate::modules::enrich::dto as enrich_dto;
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/tracks", get(search))
        .route(
            "/tracks/{track_urn}",
            get(get_by_id).put(update_track).delete(delete_track),
        )
        .route("/tracks/{track_urn}/stream", get(proxy_stream))
        .route(
            "/tracks/{track_urn}/comments",
            get(get_comments).post(create_comment),
        )
        .route("/tracks/{track_urn}/sharing", put(set_track_sharing))
        .route("/tracks/{track_urn}/favoriters", get(get_favoriters))
        .route("/tracks/{track_urn}/reposters", get(get_reposters))
        .route("/tracks/{track_urn}/related", get(get_related))
}

#[derive(Debug, Clone, Deserialize)]
struct SharingBody {
    sharing: String,
}

#[derive(Clone, Deserialize)]
struct SecretTokenQuery {
    #[serde(default)]
    secret_token: Option<String>,
}

#[derive(Clone, Deserialize)]
struct StreamProxyQuery {
    #[serde(default)]
    secret_token: Option<String>,
    #[serde(default)]
    hq: Option<String>,
}

async fn search(
    State(st): State<AppState>,
    ctx: SessionCtx,
    Query(p): Query<PaginationQuery>,
    Query(q): Query<crate::modules::search::query::TrackSearchQuery>,
) -> AppResult<Json<ListPageResult<Value>>> {
    let (page, limit) = p.resolved();
    let mut result = st.search.tracks(&q, page, limit).await?;
    crate::modules::likes::cold::apply_user_favorite_flag(
        &st.pg,
        &ctx.sc_user_id,
        &mut result.collection,
    )
    .await?;
    Ok(Json(result))
}

async fn get_by_id(
    State(st): State<AppState>,
    ctx: SessionCtx,
    Path(track_urn): Path<String>,
    Query(s): Query<SecretTokenQuery>,
) -> AppResult<Json<Value>> {
    let mut params: Vec<(String, String)> = Vec::new();
    if let Some(t) = s.secret_token {
        params.push(("secret_token".into(), t));
    }
    let mut track = st
        .tracks
        .get_by_id(ctx.session_id, &ctx.sc_user_id, &track_urn, &params)
        .await?;
    enrich_dto::apply_to_track(&st.pg, &mut track).await?;
    Ok(Json(track))
}

async fn update_track(
    State(st): State<AppState>,
    ctx: SessionCtx,
    Path(track_urn): Path<String>,
    Json(body): Json<Value>,
) -> AppResult<Json<Value>> {
    Ok(Json(
        st.tracks.update(&ctx.sc_user_id, &track_urn, &body).await?,
    ))
}

async fn delete_track(
    State(st): State<AppState>,
    ctx: SessionCtx,
    Path(track_urn): Path<String>,
) -> AppResult<Json<Value>> {
    Ok(Json(st.tracks.delete(&ctx.sc_user_id, &track_urn).await?))
}

async fn set_track_sharing(
    State(st): State<AppState>,
    ctx: SessionCtx,
    Path(track_urn): Path<String>,
    Json(body): Json<SharingBody>,
) -> AppResult<Json<Value>> {
    Ok(Json(
        st.tracks
            .set_sharing(&ctx.sc_user_id, &track_urn, &body.sharing)
            .await?,
    ))
}

async fn proxy_stream(
    State(st): State<AppState>,
    ctx: SessionCtx,
    Path(track_urn): Path<String>,
    Query(q): Query<StreamProxyQuery>,
) -> AppResult<Response> {
    st.tracks
        .ensure_read_access(&ctx.sc_user_id, &track_urn, q.secret_token.is_some())
        .await?;
    if q.secret_token.is_some() {
        ctx.access_token().await?;
    }
    let high_quality = q.hq.as_deref() == Some("true");
    let ticket = st
        .config
        .streaming
        .ticket_key
        .issue(
            ctx.session_id,
            &track_urn,
            q.secret_token.as_deref(),
            high_quality,
            Duration::from_secs(120),
        )
        .map_err(|error| match error {
            stream_ticket::StreamTicketError::SecretTooLong => {
                AppError::bad_request("secret_token is too long")
            }
            error => AppError::internal(format!("failed to issue stream ticket: {error}")),
        })?;
    let mut url = st.config.streaming.service_url.clone();
    url.path_segments_mut()
        .map_err(|_| AppError::internal("streaming service URL cannot be a base URL"))?
        .extend(["stream", &track_urn]);
    url.query_pairs_mut().append_pair("ticket", &ticket);
    Ok(Redirect::temporary(url.as_str()).into_response())
}

async fn get_comments(
    State(st): State<AppState>,
    ctx: SessionCtx,
    Path(track_urn): Path<String>,
    Query(p): Query<PaginationQuery>,
) -> AppResult<Json<CollectionPage>> {
    let (page, limit) = p.resolved();
    Ok(Json(
        st.tracks
            .get_comments(&ctx.sc_user_id, &track_urn, page, limit)
            .await?,
    ))
}

async fn create_comment(
    State(st): State<AppState>,
    ctx: SessionCtx,
    Path(track_urn): Path<String>,
    Json(body): Json<Value>,
) -> AppResult<Json<Value>> {
    Ok(Json(
        st.tracks
            .create_comment(&ctx.sc_user_id, &track_urn, &body)
            .await?,
    ))
}

async fn get_favoriters(
    State(st): State<AppState>,
    ctx: SessionCtx,
    Path(track_urn): Path<String>,
    Query(p): Query<PaginationQuery>,
) -> AppResult<Json<CollectionPage>> {
    let (page, limit) = p.resolved();
    Ok(Json(
        st.tracks
            .get_favoriters(&ctx.sc_user_id, &track_urn, page, limit)
            .await?,
    ))
}

async fn get_reposters(
    State(st): State<AppState>,
    ctx: SessionCtx,
    Path(track_urn): Path<String>,
    Query(p): Query<PaginationQuery>,
) -> AppResult<Json<CollectionPage>> {
    let (page, limit) = p.resolved();
    Ok(Json(
        st.tracks
            .get_reposters(&ctx.sc_user_id, &track_urn, page, limit)
            .await?,
    ))
}

async fn get_related(
    State(st): State<AppState>,
    ctx: SessionCtx,
    Path(track_urn): Path<String>,
    Query(p): Query<PaginationQuery>,
) -> AppResult<Json<ListPageResult<Value>>> {
    let (page, limit) = p.resolved();
    let sc_track_id = st
        .tracks
        .readable_sc_track_id(&ctx.sc_user_id, &track_urn)
        .await?;
    let mut result = st
        .recommendations
        .related_tracks(&ctx.sc_user_id, &sc_track_id, page, limit)
        .await?;
    enrich_dto::apply_to_tracks(&st.pg, &mut result.collection).await?;
    Ok(Json(result))
}
