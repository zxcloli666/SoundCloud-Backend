use axum::Json;
use axum::extract::{Path, State};
use axum::response::Response;
use backend_contracts::admin_video::{AdminVideoCommand, VideoSource};
use serde::Deserialize;

use super::forward::forward_video;
use crate::common::admin::AdminAuth;
use crate::error::AppResult;
use crate::state::AppState;

#[derive(Deserialize)]
pub struct SaveBody {
    pub source: VideoSource,
    pub secret: String,
}

#[tracing::instrument(skip_all)]
pub async fn list(_: AdminAuth, State(state): State<AppState>) -> AppResult<Response> {
    forward_video(&state, AdminVideoCommand::List).await
}

#[tracing::instrument(skip_all)]
pub async fn save(
    _: AdminAuth,
    State(state): State<AppState>,
    Path(name): Path<String>,
    Json(body): Json<SaveBody>,
) -> AppResult<Response> {
    let command = AdminVideoCommand::Save {
        source: body.source,
        name,
        secret: body.secret,
    };
    forward_video(&state, command).await
}

#[tracing::instrument(skip_all)]
pub async fn check(
    _: AdminAuth,
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> AppResult<Response> {
    forward_video(&state, AdminVideoCommand::Check { name }).await
}
