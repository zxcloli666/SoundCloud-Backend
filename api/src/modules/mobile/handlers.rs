use std::sync::Arc;

use axum::extract::{Query, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post, put};
use axum::{Json, Router};
use serde::Serialize;

use crate::common::session::SessionCtx;
use crate::error::AppResult;
use crate::state::AppState;

use super::dto::{LinkCallback, LinkStatusQuery, UqChoice};
use super::limits::Action;
use super::service::MobileService;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/auth/mobile", delete(unlink))
        .route("/auth/mobile/link", post(start))
        .route("/auth/mobile/link/callback", post(callback))
        .route("/auth/mobile/link/status", get(status))
        .route("/me/mobile", get(overview))
        .route("/me/mobile/uq", put(choose_uq))
}

#[tracing::instrument(skip_all)]
async fn start(State(mobile): State<Arc<MobileService>>, ctx: SessionCtx) -> AppResult<Response> {
    mobile.limits.admit(Action::Start, ctx.session_id)?;
    let started = mobile.start(ctx.session_id, &ctx.sc_user_id).await?;
    Ok(no_store(started))
}

#[tracing::instrument(skip_all)]
async fn callback(
    State(mobile): State<Arc<MobileService>>,
    ctx: SessionCtx,
    Json(body): Json<LinkCallback>,
) -> AppResult<Response> {
    mobile.limits.admit(Action::Callback, ctx.session_id)?;
    let status = mobile
        .callback(
            ctx.session_id,
            &ctx.sc_user_id,
            body.id,
            &body.code,
            &body.state,
        )
        .await?;
    Ok(no_store(status))
}

#[tracing::instrument(skip_all)]
async fn status(
    State(mobile): State<Arc<MobileService>>,
    ctx: SessionCtx,
    Query(query): Query<LinkStatusQuery>,
) -> AppResult<Response> {
    mobile.limits.admit(Action::Poll, ctx.session_id)?;
    let status = mobile.status(ctx.session_id, query.id).await?;
    Ok(no_store(status))
}

#[tracing::instrument(skip_all)]
async fn unlink(
    State(mobile): State<Arc<MobileService>>,
    ctx: SessionCtx,
) -> AppResult<StatusCode> {
    mobile.limits.admit(Action::Change, ctx.session_id)?;
    mobile.unlink(&ctx.sc_user_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[tracing::instrument(skip_all)]
async fn overview(
    State(mobile): State<Arc<MobileService>>,
    ctx: SessionCtx,
) -> AppResult<Response> {
    mobile.limits.admit(Action::Read, ctx.session_id)?;
    let overview = mobile.overview(ctx.session_id, &ctx.sc_user_id).await?;
    Ok(no_store(overview))
}

#[tracing::instrument(skip_all)]
async fn choose_uq(
    State(mobile): State<Arc<MobileService>>,
    ctx: SessionCtx,
    Json(choice): Json<UqChoice>,
) -> AppResult<Response> {
    mobile.limits.admit(Action::Change, ctx.session_id)?;
    let overview = mobile
        .choose_uq(ctx.session_id, &ctx.sc_user_id, choice.opt_in)
        .await?;
    Ok(no_store(overview))
}

fn no_store<T: Serialize>(value: T) -> Response {
    let mut response = Json(value).into_response();
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("no-store, private"),
    );
    response
}
