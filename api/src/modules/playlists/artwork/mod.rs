use std::sync::Arc;

use axum::http::StatusCode;
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use serde_json::{Value, json};
use sqlx::PgPool;
use tokio::sync::Semaphore;
use uuid::Uuid;

use crate::common::sc_ids::extract_sc_id;
use crate::error::{AppError, AppResult};
use crate::modules::sync_queue::SyncQueueService;

use super::mutations::target_urn;
use super::service::assert_owner;

mod normalize;
mod store;

use normalize::Refusal;
pub use store::ArtworkStore;

pub const ACTION_TYPE: &str = "playlist_artwork";
pub const IMAGE_MAX_BYTES: usize = 8 * 1024 * 1024;
pub const BODY_MAX_BYTES: usize = IMAGE_MAX_BYTES / 3 * 4 + 64 * 1024;
const NORMALIZING_AT_ONCE: usize = 2;

pub(super) struct PlaylistArtwork {
    pg: PgPool,
    queue: Arc<SyncQueueService>,
    store: Option<ArtworkStore>,
    normalizing: Semaphore,
}

impl PlaylistArtwork {
    pub(super) fn new(
        pg: PgPool,
        queue: Arc<SyncQueueService>,
        store: Option<ArtworkStore>,
    ) -> Self {
        Self {
            pg,
            queue,
            store,
            normalizing: Semaphore::new(NORMALIZING_AT_ONCE),
        }
    }

    pub(super) async fn change(
        &self,
        user: &str,
        playlist: &str,
        image_data: &str,
    ) -> AppResult<Value> {
        let target = target_urn(playlist)?;
        assert_owner(&self.pg, user, &target).await?;
        let store = self.store.as_ref().ok_or_else(unavailable)?;
        let image = STANDARD
            .decode(image_data.trim())
            .map_err(|_| refused(Refusal::Unreadable))?;
        if image.len() > IMAGE_MAX_BYTES {
            return Err(refused(Refusal::TooLarge));
        }
        let jpeg = self.normalized(image).await?;
        let key = format!("{}/{}.jpg", extract_sc_id(&target), Uuid::now_v7());
        store.put(&key, jpeg).await.map_err(|error| {
            tracing::error!(%error, %target, "playlist artwork was not staged");
            unavailable()
        })?;
        self.queue
            .enqueue(user, ACTION_TYPE, &target, Some(&json!({"key": key})))
            .await?;
        Ok(json!({"status": "queued", "actionType": ACTION_TYPE, "targetUrn": target}))
    }

    async fn normalized(&self, image: Vec<u8>) -> AppResult<Vec<u8>> {
        let _turn = self
            .normalizing
            .acquire()
            .await
            .map_err(|_| unavailable())?;
        tokio::task::spawn_blocking(move || normalize::square_jpeg(&image))
            .await
            .map_err(|error| AppError::internal(error.to_string()))?
            .map_err(refused)
    }
}

fn unavailable() -> AppError {
    AppError::coded(
        StatusCode::SERVICE_UNAVAILABLE,
        "artwork_unavailable",
        "playlist covers cannot be changed right now",
    )
}

fn refused(refusal: Refusal) -> AppError {
    match refusal {
        Refusal::Unsupported => AppError::coded(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "artwork_unsupported",
            "the cover must be a JPEG, PNG or WebP image",
        ),
        Refusal::Unreadable => AppError::coded(
            StatusCode::BAD_REQUEST,
            "artwork_invalid",
            "the cover image cannot be read",
        ),
        Refusal::TooLarge => AppError::coded(
            StatusCode::PAYLOAD_TOO_LARGE,
            "artwork_too_large",
            "the cover image is too large",
        ),
        Refusal::TooSmall => AppError::coded(
            StatusCode::UNPROCESSABLE_ENTITY,
            "artwork_too_small",
            format!(
                "the cover must be at least {} px on each side",
                normalize::SIDE_MIN
            ),
        ),
    }
}

#[cfg(test)]
#[path = "artwork_tests.rs"]
mod tests;
