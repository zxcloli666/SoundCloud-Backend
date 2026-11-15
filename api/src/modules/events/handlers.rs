use axum::extract::State;
use axum::routing::post;
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::common::session::SessionCtx;
use crate::error::AppResult;
use crate::modules::events::seek::{SEEK_EVENT, Seek};
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new().route("/events", post(record))
}

#[derive(Debug, Deserialize)]
struct RecordEventDto {
    #[serde(rename = "scTrackId")]
    sc_track_id: String,
    #[serde(rename = "eventType")]
    event_type: String,
    #[serde(rename = "positionPct", default)]
    position_pct: Option<f32>,
    #[serde(rename = "positionMs", default)]
    position_ms: Option<i64>,
    #[serde(rename = "fromMs", default)]
    from_ms: Option<i64>,
    #[serde(default)]
    source: Option<String>,
}

async fn record(
    State(st): State<AppState>,
    ctx: SessionCtx,
    Json(body): Json<RecordEventDto>,
) -> AppResult<Json<Value>> {
    if body.event_type == SEEK_EVENT {
        let seek = Seek::parse(body.position_ms, body.from_ms, body.source.as_deref())?;
        st.events
            .record_seek(&ctx.sc_user_id, &body.sc_track_id, seek)
            .await?;
        return Ok(Json(json!({ "ok": true })));
    }
    st.events
        .record(
            &ctx.sc_user_id,
            &body.sc_track_id,
            &body.event_type,
            body.position_pct,
        )
        .await?;
    Ok(Json(json!({ "ok": true })))
}
