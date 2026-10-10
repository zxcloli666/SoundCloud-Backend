use std::time::Duration;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::routing::{get, post, put};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::common::admission::Endpoint;
use crate::common::session::SessionCtx;
use crate::error::{AppError, AppResult};
use crate::modules::rooms::identity::account_profile;
use crate::modules::rooms::limits::admit;
use crate::modules::rooms::listing::PublicRoom;
use crate::modules::rooms::model::{PlaybackUpdate, Profile, normalize_code};
use crate::modules::rooms::public::blocked_hosts;
use crate::modules::rooms::service::RoomView;
use crate::state::AppState;

const LONG_POLL_HOLD: Duration = Duration::from_secs(20);

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/rooms", post(create))
        .route("/rooms/public", get(list_public))
        .route("/rooms/public/{host_id}/members", post(join_public))
        .route("/rooms/{code}", get(show))
        .route("/rooms/{code}/visibility", put(visibility))
        .route("/rooms/{code}/members", post(join))
        .route("/rooms/{code}/members/me", axum::routing::delete(leave))
        .route("/rooms/{code}/playback", put(playback))
        .route("/rooms/{code}/ready", put(ready))
}

#[derive(Debug, Clone, Deserialize)]
struct WaitQuery {
    #[serde(default)]
    since: Option<u64>,
    #[serde(default)]
    follow: bool,
}

#[derive(Debug, Clone, Deserialize)]
struct CreateBody {
    #[serde(flatten)]
    profile: Profile,
    #[serde(default)]
    public: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReadyBody {
    track_urn: String,
}

#[derive(Debug, Clone, Deserialize)]
struct VisibilityBody {
    public: bool,
}

fn code_of(raw: &str) -> AppResult<String> {
    normalize_code(raw).ok_or_else(|| AppError::not_found("Room not found"))
}

async fn create(
    State(st): State<AppState>,
    ctx: SessionCtx,
    Json(body): Json<CreateBody>,
) -> AppResult<(StatusCode, Json<RoomView>)> {
    admit(&st, &ctx, Endpoint::RoomCreate).await?;
    let profile = account_profile(&st.pg, &ctx.sc_user_id, body.profile).await?;
    let room = st
        .rooms
        .create(&ctx.sc_user_id, &profile, body.public)
        .await?;
    Ok((StatusCode::CREATED, Json(room)))
}

async fn show(
    State(st): State<AppState>,
    ctx: SessionCtx,
    Path(code): Path<String>,
    Query(q): Query<WaitQuery>,
) -> AppResult<Json<RoomView>> {
    let code = code_of(&code)?;
    Ok(Json(
        st.rooms
            .wait(&code, &ctx.sc_user_id, q.since, LONG_POLL_HOLD, q.follow)
            .await?,
    ))
}

async fn join(
    State(st): State<AppState>,
    ctx: SessionCtx,
    Path(code): Path<String>,
    Json(profile): Json<Profile>,
) -> AppResult<Json<RoomView>> {
    let code = code_of(&code)?;
    admit(&st, &ctx, Endpoint::RoomJoin).await?;
    let profile = account_profile(&st.pg, &ctx.sc_user_id, profile).await?;
    Ok(Json(st.rooms.join(&code, &ctx.sc_user_id, &profile).await?))
}

async fn leave(
    State(st): State<AppState>,
    ctx: SessionCtx,
    Path(code): Path<String>,
) -> AppResult<StatusCode> {
    let code = code_of(&code)?;
    st.rooms.leave(&code, &ctx.sc_user_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn playback(
    State(st): State<AppState>,
    ctx: SessionCtx,
    Path(code): Path<String>,
    Json(update): Json<PlaybackUpdate>,
) -> AppResult<Json<RoomView>> {
    let code = code_of(&code)?;
    Ok(Json(
        st.rooms
            .set_playback(&code, &ctx.sc_user_id, update)
            .await?,
    ))
}

async fn ready(
    State(st): State<AppState>,
    ctx: SessionCtx,
    Path(code): Path<String>,
    Json(body): Json<ReadyBody>,
) -> AppResult<Json<RoomView>> {
    let code = code_of(&code)?;
    Ok(Json(
        st.rooms
            .mark_ready(&code, &ctx.sc_user_id, &body.track_urn)
            .await?,
    ))
}

async fn list_public(State(st): State<AppState>, ctx: SessionCtx) -> AppResult<Json<Value>> {
    admit(&st, &ctx, Endpoint::RoomList).await?;
    let rooms = st.rooms.public_rooms().await?;
    let blocked = blocked_hosts(&st.pg, &ctx.sc_user_id).await?;
    let collection: Vec<&PublicRoom> = rooms
        .iter()
        .filter(|room| room.host_id != ctx.sc_user_id && !blocked.contains(&room.host_id))
        .collect();
    Ok(Json(json!({ "collection": collection })))
}

async fn join_public(
    State(st): State<AppState>,
    ctx: SessionCtx,
    Path(host_id): Path<String>,
    Json(profile): Json<Profile>,
) -> AppResult<Json<RoomView>> {
    admit(&st, &ctx, Endpoint::RoomJoin).await?;
    let profile = account_profile(&st.pg, &ctx.sc_user_id, profile).await?;
    Ok(Json(
        st.rooms
            .join_public(&host_id, &ctx.sc_user_id, &profile)
            .await?,
    ))
}

async fn visibility(
    State(st): State<AppState>,
    ctx: SessionCtx,
    Path(code): Path<String>,
    Json(body): Json<VisibilityBody>,
) -> AppResult<Json<RoomView>> {
    let code = code_of(&code)?;
    Ok(Json(
        st.rooms
            .set_public(&code, &ctx.sc_user_id, body.public)
            .await?,
    ))
}
