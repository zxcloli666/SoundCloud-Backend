use axum::http::StatusCode;
use backend_contracts::lyrics_search::{LyricsSearchCandidate, LyricsSearchRequest};
use serde::Serialize;

use crate::error::{AppError, AppResult};
use crate::modules::auth::core_rpc::CoreRpc;

const MAX_QUERY_CHARS: usize = 200;
const RETRY_AFTER_SECONDS: i64 = 10;

#[derive(Debug, Serialize)]
pub struct CandidatesResponse {
    pub candidates: Vec<LyricsSearchCandidate>,
}

pub async fn search(
    core: &CoreRpc,
    query: &str,
    duration_sec: Option<i64>,
) -> AppResult<CandidatesResponse> {
    let query: String = query.trim().chars().take(MAX_QUERY_CHARS).collect();
    if query.is_empty() {
        return Err(AppError::bad_request("q is required"));
    }
    let reply = core
        .lyrics_search(&LyricsSearchRequest {
            query,
            duration_sec,
        })
        .await
        .map_err(|_| unavailable())?;
    if !reply.reachable {
        return Err(unavailable());
    }
    Ok(CandidatesResponse {
        candidates: reply.candidates,
    })
}

fn unavailable() -> AppError {
    AppError::coded(
        StatusCode::SERVICE_UNAVAILABLE,
        "lyrics_search_unavailable",
        "lyrics search is unavailable, retry shortly",
    )
    .with_retry_after(RETRY_AFTER_SECONDS)
}
