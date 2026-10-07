use axum::extract::{Path, State};
use axum::routing::{get, put};
use axum::{Json, Router};
use serde_json::{Value, json};

use crate::common::session::SessionCtx;
use crate::error::AppResult;
use crate::modules::blocked_artists::service::{self, BlockInput, BlockKind, BlockedArtist};
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/blocked-artists", get(list))
        .route("/blocked-artists/{kind}/{id}", put(block).delete(unblock))
}

async fn list(State(st): State<AppState>, ctx: SessionCtx) -> AppResult<Json<Value>> {
    let collection = service::list(&st.pg, &ctx.sc_user_id).await?;
    Ok(Json(json!({ "collection": collection })))
}

async fn block(
    State(st): State<AppState>,
    ctx: SessionCtx,
    Path((kind, id)): Path<(String, String)>,
    Json(input): Json<BlockInput>,
) -> AppResult<Json<BlockedArtist>> {
    let kind = BlockKind::parse(&kind)?;
    Ok(Json(
        service::block(&st.pg, &ctx.sc_user_id, kind, &id, &input).await?,
    ))
}

async fn unblock(
    State(st): State<AppState>,
    ctx: SessionCtx,
    Path((kind, id)): Path<(String, String)>,
) -> AppResult<Json<Value>> {
    let kind = BlockKind::parse(&kind)?;
    service::unblock(&st.pg, &ctx.sc_user_id, kind, &id).await?;
    Ok(Json(json!({ "status": "removed" })))
}
