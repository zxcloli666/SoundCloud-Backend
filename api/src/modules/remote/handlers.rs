use std::time::Duration;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use backend_contracts::remote::{
    LinkCodeRequest, LinkCodeResponse, LinksResponse, RemoteReply, WAIT_HOLD_SECS, WaitResponse,
};
use serde::Deserialize;
use uuid::Uuid;

use crate::common::admission::{AdmissionRejection, Endpoint};
use crate::common::session::SessionCtx;
use crate::error::{AppError, AppResult};
use crate::state::AppState;

const MAX_DEVICE_LEN: usize = 64;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/remote/wait", get(wait))
        .route("/remote/reply", post(reply))
        .route("/remote/link-code", post(link_code))
        .route("/remote/links", get(links))
        .route("/remote/links/{id}", delete(revoke))
}

#[derive(Debug, Deserialize)]
struct WaitQuery {
    #[serde(default)]
    since: Option<i64>,
    #[serde(default)]
    device: Option<String>,
    #[serde(default)]
    active: Option<i64>,
}

pub fn star_required() -> AppError {
    AppError::coded(
        StatusCode::FORBIDDEN,
        "star_required",
        "Star subscription required",
    )
}

async fn require_star(st: &AppState, ctx: &SessionCtx) -> AppResult<()> {
    if st.subscriptions.is_premium(&ctx.sc_user_id).await? {
        Ok(())
    } else {
        Err(star_required())
    }
}

async fn wait(
    State(st): State<AppState>,
    ctx: SessionCtx,
    Query(q): Query<WaitQuery>,
) -> AppResult<Json<WaitResponse>> {
    require_star(&st, &ctx).await?;
    let device: String = q
        .device
        .unwrap_or_default()
        .chars()
        .take(MAX_DEVICE_LEN)
        .collect();
    Ok(Json(
        st.remote
            .wait(
                &ctx.sc_user_id,
                &device,
                q.active.unwrap_or(0),
                q.since,
                Duration::from_secs(WAIT_HOLD_SECS),
            )
            .await,
    ))
}

async fn reply(
    State(st): State<AppState>,
    ctx: SessionCtx,
    Json(body): Json<RemoteReply>,
) -> AppResult<StatusCode> {
    st.remote.reply(&ctx.sc_user_id, body).await;
    Ok(StatusCode::NO_CONTENT)
}

async fn link_code(
    State(st): State<AppState>,
    ctx: SessionCtx,
    Json(body): Json<LinkCodeRequest>,
) -> AppResult<Json<LinkCodeResponse>> {
    require_star(&st, &ctx).await?;
    st.admission
        .check_session(Endpoint::VoiceLinkCode, ctx.session_id)
        .await
        .map_err(|rejection| match rejection {
            AdmissionRejection::Limited {
                retry_after_seconds,
            } => AppError::coded(
                StatusCode::TOO_MANY_REQUESTS,
                "voice_link_limited",
                "Too many link codes requested",
            )
            .with_retry_after(i64::try_from(retry_after_seconds).unwrap_or(60)),
            AdmissionRejection::Unavailable => AppError::coded(
                StatusCode::SERVICE_UNAVAILABLE,
                "voice_link_busy",
                "Link codes are busy, try again shortly",
            )
            .with_retry_after(1),
        })?;
    Ok(Json(
        st.links.create_code(&ctx.sc_user_id, body.provider).await?,
    ))
}

async fn links(State(st): State<AppState>, ctx: SessionCtx) -> AppResult<Json<LinksResponse>> {
    Ok(Json(LinksResponse {
        links: st.links.list(&ctx.sc_user_id).await?,
    }))
}

async fn revoke(
    State(st): State<AppState>,
    ctx: SessionCtx,
    Path(id): Path<Uuid>,
) -> AppResult<StatusCode> {
    if st.links.revoke(&ctx.sc_user_id, id).await? {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(AppError::not_found("Link not found"))
    }
}
