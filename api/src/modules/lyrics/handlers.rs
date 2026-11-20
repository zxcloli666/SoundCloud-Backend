use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::routing::{get, put};
use axum::{Json, Router};
use serde::Deserialize;

use crate::common::session::SessionCtx;
use crate::error::AppResult;
use crate::modules::lyrics::own::{self, OwnLyricsInput};
use crate::modules::lyrics::service::{LyricsHints, LyricsResponse, canonical_track_id};
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/lyrics/search", get(search))
        .route("/lyrics/{sc_track_id}", get(get_one))
        .route("/lyrics/{sc_track_id}/own", put(put_own).delete(delete_own))
}

#[derive(Debug, Deserialize)]
struct SearchQuery {
    #[serde(default)]
    artist: Option<String>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    duration: Option<String>,
}

fn parse_duration(d: Option<&str>) -> Option<i64> {
    let s = d?;
    let parsed: f64 = s.parse().ok()?;
    if !parsed.is_finite() {
        return None;
    }
    if parsed >= 10_000.0 {
        Some((parsed / 1000.0).round() as i64)
    } else {
        Some(parsed.round() as i64)
    }
}

async fn search(
    State(st): State<AppState>,
    _ctx: SessionCtx,
    Query(q): Query<SearchQuery>,
) -> AppResult<Json<LyricsResponse>> {
    let hints = LyricsHints {
        artist: q.artist.unwrap_or_default(),
        title: q.title.unwrap_or_default(),
        duration_sec: parse_duration(q.duration.as_deref()),
    };
    Ok(Json(st.lyrics.search_lyrics(&hints).await?))
}

async fn get_one(
    State(st): State<AppState>,
    ctx: SessionCtx,
    Path(sc_track_id): Path<String>,
) -> AppResult<Json<LyricsResponse>> {
    Ok(Json(
        st.lyrics
            .ensure_lyrics(&ctx.sc_user_id, &sc_track_id)
            .await?,
    ))
}

async fn put_own(
    State(st): State<AppState>,
    ctx: SessionCtx,
    Path(sc_track_id): Path<String>,
    Json(input): Json<OwnLyricsInput>,
) -> AppResult<Json<LyricsResponse>> {
    let sc_track_id = canonical_track_id(&sc_track_id)?;
    Ok(Json(
        own::put(&st.pg, &ctx.sc_user_id, &sc_track_id, input).await?,
    ))
}

async fn delete_own(
    State(st): State<AppState>,
    ctx: SessionCtx,
    Path(sc_track_id): Path<String>,
) -> AppResult<StatusCode> {
    let sc_track_id = canonical_track_id(&sc_track_id)?;
    own::remove(&st.pg, &ctx.sc_user_id, &sc_track_id).await?;
    Ok(StatusCode::NO_CONTENT)
}
