use std::time::Duration;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::routing::{get, post, put};
use axum::{Json, Router};
use serde::Deserialize;

use crate::common::session::SessionCtx;
use crate::error::{AppError, AppResult};
use crate::modules::rooms::model::{PlaybackUpdate, Profile, normalize_code};
use crate::modules::rooms::service::RoomView;
use crate::state::AppState;

const LONG_POLL_HOLD: Duration = Duration::from_secs(20);

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/rooms", post(create))
        .route("/rooms/{code}", get(show))
        .route("/rooms/{code}/members", post(join))
        .route("/rooms/{code}/members/me", axum::routing::delete(leave))
        .route("/rooms/{code}/playback", put(playback))
        .route("/rooms/{code}/ready", put(ready))
}

#[derive(Debug, Clone, Deserialize)]
struct WaitQuery {
    #[serde(default)]
    since: Option<u64>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReadyBody {
    track_urn: String,
}

fn code_of(raw: &str) -> AppResult<String> {
    normalize_code(raw).ok_or_else(|| AppError::not_found("Room not found"))
}

async fn create(
    State(st): State<AppState>,
    ctx: SessionCtx,
    Json(profile): Json<Profile>,
) -> AppResult<(StatusCode, Json<RoomView>)> {
    let room = st.rooms.create(&ctx.sc_user_id, &profile).await?;
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
            .wait(&code, &ctx.sc_user_id, q.since, LONG_POLL_HOLD)
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
