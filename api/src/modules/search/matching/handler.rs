use std::collections::HashMap;

use axum::body::Bytes;
use axum::extract::State;
use axum::routing::post;
use axum::{Json, Router};
use futures::{StreamExt, TryStreamExt, stream};
use serde::{Deserialize, Serialize};

use super::find::find;
use super::wanted::Wanted;
use crate::common::session::SessionCtx;
use crate::error::{AppError, AppResult};
use crate::state::AppState;

const MAX_ITEMS: usize = 100;
const PARALLEL: usize = 4;

pub fn router() -> Router<AppState> {
    Router::new().route("/tracks/match", post(match_tracks))
}

#[derive(Deserialize)]
struct MatchRequest {
    items: Vec<Item>,
}

#[derive(Deserialize)]
struct Item {
    #[serde(default)]
    artist: Option<String>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    duration_ms: Option<i64>,
}

#[derive(Serialize)]
struct MatchResponse {
    results: Vec<Matched>,
}

#[derive(Serialize)]
struct Matched {
    urn: Option<String>,
}

async fn match_tracks(
    State(st): State<AppState>,
    _ctx: SessionCtx,
    body: Bytes,
) -> AppResult<Json<MatchResponse>> {
    let request: MatchRequest = serde_json::from_slice(&body).map_err(|_| {
        AppError::bad_request("Body must be {\"items\":[{artist,title,duration_ms}]}")
    })?;
    if request.items.is_empty() || request.items.len() > MAX_ITEMS {
        return Err(AppError::bad_request("items must hold 1 to 100 entries"));
    }
    let (unique, slots) = dedupe(request.items);
    let found: Vec<Option<String>> = stream::iter(unique)
        .map(|wanted| {
            let search = st.search.clone();
            async move { find(&search, &wanted).await }
        })
        .buffered(PARALLEL)
        .try_collect()
        .await?;
    let results = slots
        .into_iter()
        .map(|slot| Matched {
            urn: found[slot].clone(),
        })
        .collect();
    Ok(Json(MatchResponse { results }))
}

fn dedupe(items: Vec<Item>) -> (Vec<Wanted>, Vec<usize>) {
    let mut seen = HashMap::new();
    let mut unique = Vec::new();
    let mut slots = Vec::new();
    for item in items {
        let wanted = Wanted::new(
            item.artist.as_deref().unwrap_or_default(),
            item.title.as_deref().unwrap_or_default(),
            item.duration_ms,
        );
        let slot = *seen.entry(wanted.key()).or_insert_with(|| {
            unique.push(wanted);
            unique.len() - 1
        });
        slots.push(slot);
    }
    (unique, slots)
}
