use axum::extract::{Path, Query, State};
use axum::http::HeaderMap;
use axum::routing::{get, put};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::Value;
use uuid::Uuid;

use crate::cache::ListPageResult;
use crate::common::pagination::PaginationQuery;
use crate::common::session::SessionCtx;
use crate::error::{AppError, AppResult};
use crate::modules::cold_refresh::collection::CollectionPage;
use crate::modules::enrich::dto as enrich_dto;
use crate::modules::playlists::EditBody;
use crate::state::AppState;

const IDEMPOTENCY_HEADER: &str = "idempotency-key";

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/playlists", get(search).post(create))
        .route(
            "/playlists/{playlist_urn}",
            get(get_by_id).put(update_playlist).delete(delete_playlist),
        )
        .route(
            "/playlists/{playlist_urn}/tracks",
            get(get_tracks).post(edit_tracks),
        )
        .route(
            "/playlists/{playlist_urn}/sharing",
            put(set_playlist_sharing),
        )
        .route("/playlists/{playlist_urn}/reposters", get(get_reposters))
}

#[derive(Debug, Clone, Deserialize)]
struct SharingBody {
    sharing: String,
}

#[derive(Clone, Deserialize)]
struct DetailQuery {
    #[serde(default)]
    secret_token: Option<String>,
    #[serde(default)]
    access: Option<String>,
    #[serde(default)]
    show_tracks: Option<String>,
}

async fn search(
    State(st): State<AppState>,
    _ctx: SessionCtx,
    Query(p): Query<PaginationQuery>,
    Query(q): Query<crate::modules::search::query::PlaylistSearchQuery>,
) -> AppResult<Json<ListPageResult<Value>>> {
    let (page, limit) = p.resolved();
    Ok(Json(st.search.playlists(&q, page, limit).await?))
}

async fn create(
    State(st): State<AppState>,
    ctx: SessionCtx,
    Json(body): Json<Value>,
) -> AppResult<Json<Value>> {
    let v = st.playlists.create(&ctx.sc_user_id, &body).await?;
    Ok(Json(v))
}

async fn get_by_id(
    State(st): State<AppState>,
    ctx: SessionCtx,
    Path(playlist_urn): Path<String>,
    Query(q): Query<DetailQuery>,
) -> AppResult<Json<Value>> {
    let mut params: Vec<(String, String)> = vec![(
        "access".into(),
        q.access
            .unwrap_or_else(|| "playable,preview,blocked".into()),
    )];
    if let Some(v) = q.secret_token {
        params.push(("secret_token".into(), v));
    }
    if let Some(v) = q.show_tracks {
        params.push(("show_tracks".into(), v));
    }
    let mut value = st
        .playlists
        .get_by_id(ctx.session_id, &ctx.sc_user_id, &playlist_urn, &params)
        .await?;
    if let Some(arr) = value.get_mut("tracks").and_then(|v| v.as_array_mut()) {
        enrich_dto::apply_to_tracks(&st.pg, arr.as_mut_slice()).await?;
    }
    let mut single = vec![value];
    crate::modules::likes::cold::apply_user_favorite_flag_to_playlists(
        &st.pg,
        &ctx.sc_user_id,
        &mut single,
    )
    .await?;
    Ok(Json(single.into_iter().next().unwrap_or(Value::Null)))
}

#[derive(Debug, Clone, Deserialize)]
struct ReplaceQuery {
    #[serde(default)]
    replace: Option<String>,
}

async fn update_playlist(
    State(st): State<AppState>,
    ctx: SessionCtx,
    Path(playlist_urn): Path<String>,
    Query(q): Query<ReplaceQuery>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> AppResult<Json<Value>> {
    let idempotency_key = idempotency_key(&headers)?;
    let replace = q.replace.as_deref() == Some("true");
    let value = st
        .playlists
        .update(
            &ctx.sc_user_id,
            &playlist_urn,
            &body,
            replace,
            idempotency_key,
        )
        .await?;
    Ok(Json(value))
}

async fn edit_tracks(
    State(st): State<AppState>,
    ctx: SessionCtx,
    Path(playlist_urn): Path<String>,
    Query(p): Query<PaginationQuery>,
    headers: HeaderMap,
    Json(body): Json<EditBody>,
) -> AppResult<Json<crate::modules::playlists::PlaylistTracksPage>> {
    let idempotency_key = idempotency_key(&headers)?;
    let request = body.into_request()?;
    let (page, limit) = p.resolved();
    let mut result = st
        .playlists
        .edit_tracks(
            &ctx.sc_user_id,
            &playlist_urn,
            request,
            idempotency_key,
            page,
            limit,
        )
        .await?;
    enrich_dto::apply_to_tracks(&st.pg, &mut result.page.collection).await?;
    Ok(Json(result))
}

fn idempotency_key(headers: &HeaderMap) -> AppResult<Uuid> {
    let Some(value) = headers.get(IDEMPOTENCY_HEADER) else {
        return Ok(Uuid::now_v7());
    };
    value
        .to_str()
        .ok()
        .and_then(|value| Uuid::parse_str(value.trim()).ok())
        .ok_or_else(|| AppError::bad_request("Idempotency-Key must be a UUID"))
}

async fn set_playlist_sharing(
    State(st): State<AppState>,
    ctx: SessionCtx,
    Path(playlist_urn): Path<String>,
    Json(body): Json<SharingBody>,
) -> AppResult<Json<Value>> {
    let v = st
        .playlists
        .set_sharing(&ctx.sc_user_id, &playlist_urn, &body.sharing)
        .await?;
    Ok(Json(v))
}

async fn delete_playlist(
    State(st): State<AppState>,
    ctx: SessionCtx,
    Path(playlist_urn): Path<String>,
) -> AppResult<Json<Value>> {
    let v = st.playlists.delete(&ctx.sc_user_id, &playlist_urn).await?;
    Ok(Json(v))
}

async fn get_tracks(
    State(st): State<AppState>,
    ctx: SessionCtx,
    Path(playlist_urn): Path<String>,
    Query(p): Query<PaginationQuery>,
) -> AppResult<Json<crate::modules::playlists::PlaylistTracksPage>> {
    let (page, limit) = p.resolved();
    let mut result = st
        .playlists
        .get_tracks(&ctx.sc_user_id, &playlist_urn, page, limit)
        .await?;
    enrich_dto::apply_to_tracks(&st.pg, &mut result.page.collection).await?;
    Ok(Json(result))
}

async fn get_reposters(
    State(st): State<AppState>,
    ctx: SessionCtx,
    Path(playlist_urn): Path<String>,
    Query(p): Query<PaginationQuery>,
) -> AppResult<Json<CollectionPage>> {
    let (page, limit) = p.resolved();
    Ok(Json(
        st.playlists
            .get_reposters(&ctx.sc_user_id, &playlist_urn, page, limit)
            .await?,
    ))
}
