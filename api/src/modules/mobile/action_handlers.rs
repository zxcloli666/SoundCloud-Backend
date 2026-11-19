use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, Query, State};
use serde::Deserialize;
use serde_json::Value;

use crate::common::session::SessionCtx;
use crate::error::AppResult;
use crate::modules::sync_queue::SyncQueueService;

use super::limits::Action;
use super::service::MobileService;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ValueQuery {
    value_urn: Option<String>,
}

#[tracing::instrument(skip_all)]
pub(super) async fn set_reaction(
    State(mobile): State<Arc<MobileService>>,
    State(queue): State<Arc<SyncQueueService>>,
    ctx: SessionCtx,
    Path((track_urn, second)): Path<(String, u64)>,
    Query(query): Query<ValueQuery>,
) -> AppResult<Json<Value>> {
    mobile.limits.admit(Action::Change, ctx.session_id)?;
    let queued = mobile
        .set_reaction(
            &queue,
            &ctx.sc_user_id,
            &track_urn,
            second,
            query.value_urn.as_deref(),
        )
        .await?;
    Ok(Json(queued))
}

#[tracing::instrument(skip_all)]
pub(super) async fn remove_reaction(
    State(mobile): State<Arc<MobileService>>,
    State(queue): State<Arc<SyncQueueService>>,
    ctx: SessionCtx,
    Path((track_urn, second)): Path<(String, u64)>,
    Query(query): Query<ValueQuery>,
) -> AppResult<Json<Value>> {
    mobile.limits.admit(Action::Change, ctx.session_id)?;
    let queued = mobile
        .remove_reaction(
            &queue,
            &ctx.sc_user_id,
            &track_urn,
            second,
            query.value_urn.as_deref(),
        )
        .await?;
    Ok(Json(queued))
}

#[tracing::instrument(skip_all)]
pub(super) async fn delete_comment(
    State(mobile): State<Arc<MobileService>>,
    State(queue): State<Arc<SyncQueueService>>,
    ctx: SessionCtx,
    Path(comment_urn): Path<String>,
) -> AppResult<Json<Value>> {
    mobile.limits.admit(Action::Change, ctx.session_id)?;
    let queued = mobile
        .delete_comment(&queue, &ctx.sc_user_id, &comment_urn)
        .await?;
    Ok(Json(queued))
}
