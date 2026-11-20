use backend_contracts::LIBRARY_SYNC_PRIORITY;
use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::PgPool;

use crate::common::sc_ids::extract_sc_id;
use crate::error::AppResult;

use super::service::{LIKED_PLAYLISTS, LIKED_TRACKS, OWNED_PLAYLISTS};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LibrarySync {
    pub status: &'static str,
    pub available_at: DateTime<Utc>,
}

pub(super) async fn state(
    pool: &PgPool,
    sc_user_id: &str,
    cooldown_sec: u64,
) -> AppResult<LibrarySync> {
    let available_at = sqlx::query_file_scalar!(
        "queries/cold_refresh/library_sync_state.sql",
        extract_sc_id(sc_user_id),
        cooldown_sec as f64
    )
    .fetch_optional(pool)
    .await?;
    let now = Utc::now();
    Ok(match available_at {
        Some(available_at) if available_at > now => LibrarySync {
            status: "cooldown",
            available_at,
        },
        _ => LibrarySync {
            status: "ready",
            available_at: now,
        },
    })
}

pub(super) async fn request(
    pool: &PgPool,
    sc_user_id: &str,
    cooldown_sec: u64,
) -> AppResult<LibrarySync> {
    let claimed = sqlx::query_file_scalar!(
        "queries/cold_refresh/library_sync_claim.sql",
        extract_sc_id(sc_user_id),
        cooldown_sec as f64
    )
    .fetch_optional(pool)
    .await?;
    let Some(available_at) = claimed else {
        return state(pool, sc_user_id, cooldown_sec).await;
    };
    for collection in [LIKED_TRACKS, LIKED_PLAYLISTS, OWNED_PLAYLISTS] {
        super::collection::sync_now(pool, collection.kind, sc_user_id, LIBRARY_SYNC_PRIORITY)
            .await?;
    }
    Ok(LibrarySync {
        status: "queued",
        available_at,
    })
}
