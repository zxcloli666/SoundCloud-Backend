mod form;
mod gate;

use std::time::Duration;

use axum::extract::{DefaultBodyLimit, Multipart, State};
use axum::http::StatusCode;
use axum::routing::post;
use axum::{Json, Router};
use catalog_ingest::TrackPriority;
use serde_json::Value;
use tower_http::timeout::TimeoutLayer;

use crate::common::sc_ids::{extract_sc_id, user_id_variants};
use crate::common::session::SessionCtx;
use crate::error::AppResult;
use crate::state::AppState;

pub use gate::UploadGate;

const UPLOAD_DEADLINE: Duration = Duration::from_secs(45 * 60);

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/tracks/upload", post(upload_track))
        .layer(DefaultBodyLimit::max(form::BODY_MAX_BYTES))
        .layer(TimeoutLayer::with_status_code(
            StatusCode::GATEWAY_TIMEOUT,
            UPLOAD_DEADLINE,
        ))
}

async fn upload_track(
    State(st): State<AppState>,
    ctx: SessionCtx,
    multipart: Multipart,
) -> AppResult<Json<Value>> {
    let _ticket = st.tracks.uploads().enter(&ctx.sc_user_id)?;
    let parsed = form::read(multipart).await?;
    let token = ctx.access_token().await?;
    let track = st.tracks.sc().upload_track(&token, &parsed.upload).await?;
    drop(parsed);
    remember_upload(&st, &ctx.sc_user_id, &track).await;
    Ok(Json(track))
}

async fn remember_upload(st: &AppState, sc_user_id: &str, track: &Value) {
    let Some(sc_track_id) = track
        .get("urn")
        .and_then(Value::as_str)
        .and_then(crate::common::sc_ids::normalize_sc_track_id)
    else {
        tracing::warn!("uploaded track came back without an urn");
        return;
    };
    let ingested = async {
        let observation = catalog_ingest::Observation::begin(&st.pg).await?;
        st.indexing
            .ingest_track_from_sc(track, TrackPriority::FreshDrop, observation)
            .await?;
        sqlx::query_file!(
            "queries/tracks/service/insert_owned.sql",
            extract_sc_id(sc_user_id),
            &sc_track_id,
            &user_id_variants(sc_user_id)
        )
        .execute(&st.pg)
        .await?;
        AppResult::Ok(())
    };
    if let Err(error) = ingested.await {
        tracing::warn!(track = %sc_track_id, %error, "uploaded track not mirrored yet");
    }
}

#[cfg(test)]
#[path = "../upload_tests.rs"]
mod tests;
