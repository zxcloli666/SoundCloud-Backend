use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};

use crate::common::sc_ids::{EntityKind, require_ref};
use crate::common::session::SessionCtx;
use crate::error::AppResult;
use crate::modules::cold_refresh::entity::want_track;
use crate::modules::likes::LikesOrderRequest;
use crate::modules::recommendations::smart_wave::graph::forget_user_affinity;
use crate::modules::superlikes::order;
use crate::modules::superlikes::page::{self, SuperlikesPage};
use crate::modules::superlikes::service::{self, Quota};
use crate::modules::tracks::TrackPriority;
use crate::state::AppState;

const PAGE_DEFAULT: i64 = 50;
const PAGE_MAX: i64 = 200;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/superlikes", get(list))
        .route("/superlikes/quota", get(quota))
        .route("/superlikes/order", post(reorder))
        .route("/superlikes/{track_urn}", post(add).delete(remove))
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct SuperlikeAnswer {
    pub superliked: bool,
    #[serde(flatten)]
    pub quota: Quota,
}

#[derive(Debug, Deserialize)]
struct PageQuery {
    #[serde(default)]
    limit: Option<String>,
    #[serde(default)]
    offset: Option<String>,
    #[serde(default)]
    cursor: Option<String>,
}

async fn add(
    State(st): State<AppState>,
    ctx: SessionCtx,
    Path(track_urn): Path<String>,
) -> AppResult<Json<SuperlikeAnswer>> {
    let track = require_ref(EntityKind::Track, &track_urn)?;
    let sc_track_id = track.sc_id();
    let premium = st.subscriptions.is_premium(&ctx.sc_user_id).await?;
    let done = service::superlike(&st.pg, &st.sync_queue, &ctx.sc_user_id, &track, premium).await?;
    want_track(&st.pg, &sc_track_id, TrackPriority::Like, &ctx.sc_user_id).await?;
    if done.newly_liked {
        st.events
            .record(&ctx.sc_user_id, &sc_track_id, "like", None)
            .await?;
    }
    forget_user_affinity(&st.recommendations.redis, &ctx.sc_user_id).await;
    Ok(Json(SuperlikeAnswer {
        superliked: true,
        quota: done.quota,
    }))
}

async fn remove(
    State(st): State<AppState>,
    ctx: SessionCtx,
    Path(track_urn): Path<String>,
) -> AppResult<Json<SuperlikeAnswer>> {
    let track = require_ref(EntityKind::Track, &track_urn)?;
    service::forget(&st.pg, &ctx.sc_user_id, &track.sc_id()).await?;
    forget_user_affinity(&st.recommendations.redis, &ctx.sc_user_id).await;
    let premium = st.subscriptions.is_premium(&ctx.sc_user_id).await?;
    Ok(Json(SuperlikeAnswer {
        superliked: false,
        quota: service::quota(&st.pg, &ctx.sc_user_id, premium).await?,
    }))
}

async fn reorder(
    State(st): State<AppState>,
    ctx: SessionCtx,
    Json(request): Json<LikesOrderRequest>,
) -> AppResult<StatusCode> {
    order::apply(&st.pg, &ctx.sc_user_id, request).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn quota(State(st): State<AppState>, ctx: SessionCtx) -> AppResult<Json<Quota>> {
    let premium = st.subscriptions.is_premium(&ctx.sc_user_id).await?;
    Ok(Json(
        service::quota(&st.pg, &ctx.sc_user_id, premium).await?,
    ))
}

async fn list(
    State(st): State<AppState>,
    ctx: SessionCtx,
    Query(q): Query<PageQuery>,
) -> AppResult<Json<SuperlikesPage>> {
    if q.cursor.is_some() && q.offset.is_none() {
        return Ok(Json(SuperlikesPage {
            collection: Vec::new(),
            next_href: None,
        }));
    }
    let limit = q
        .limit
        .as_deref()
        .and_then(|raw| raw.parse::<i64>().ok())
        .unwrap_or(PAGE_DEFAULT)
        .clamp(1, PAGE_MAX);
    let offset = q
        .offset
        .as_deref()
        .and_then(|raw| raw.parse::<i64>().ok())
        .unwrap_or(0)
        .max(0);
    Ok(Json(
        page::page(&st.pg, &ctx.sc_user_id, limit, offset).await?,
    ))
}
