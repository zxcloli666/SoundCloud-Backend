use std::sync::Arc;

use serde_json::{Value, json};
use sqlx::PgPool;

use crate::common::sc_ids::extract_sc_id;
use crate::error::AppResult;
use crate::modules::events::EventsService;
use crate::modules::indexing::IndexingService;
use crate::modules::sync_queue::SyncQueueService;
use crate::modules::sync_queue::mirror::{LIKES_PLAYLISTS, LIKES_TRACKS};
use crate::modules::tracks::TrackPriority;

pub struct LikesService {
    pg: PgPool,
    sync_queue: Arc<SyncQueueService>,
    indexing: Arc<IndexingService>,
    events: Arc<EventsService>,
}

impl LikesService {
    pub fn new(
        pg: PgPool,
        sync_queue: Arc<SyncQueueService>,
        indexing: Arc<IndexingService>,
        events: Arc<EventsService>,
    ) -> Arc<Self> {
        Arc::new(Self {
            pg,
            sync_queue,
            indexing,
            events,
        })
    }

    pub async fn like_track(
        &self,
        sc_user_id: &str,
        track_urn: &str,
        track_data: Option<&Value>,
    ) -> AppResult<Value> {
        let sc_track_id = extract_sc_id(track_urn);
        if let Some(td) = track_data {
            self.indexing
                .ingest_track_from_sc(
                    td,
                    TrackPriority::Like,
                    catalog_ingest::Observation::UNVERIFIED,
                )
                .await?;
        }
        self.sync_queue
            .set_wanted(
                LIKES_TRACKS,
                sc_user_id,
                sc_track_id,
                "like_track",
                track_urn,
            )
            .await?;
        self.events
            .record(sc_user_id, sc_track_id, "like", None)
            .await?;
        Ok(json!({ "status": "queued", "actionType": "like_track" }))
    }

    pub async fn unlike_track(&self, sc_user_id: &str, track_urn: &str) -> AppResult<Value> {
        let sc_track_id = extract_sc_id(track_urn);
        self.sync_queue
            .clear_wanted(
                LIKES_TRACKS,
                sc_user_id,
                sc_track_id,
                "unlike_track",
                track_urn,
            )
            .await?;
        Ok(json!({ "status": "queued", "actionType": "unlike_track" }))
    }

    pub async fn like_playlist(&self, sc_user_id: &str, playlist_urn: &str) -> AppResult<Value> {
        self.sync_queue
            .set_wanted(
                LIKES_PLAYLISTS,
                sc_user_id,
                playlist_urn,
                "like_playlist",
                playlist_urn,
            )
            .await?;
        Ok(json!({ "status": "queued", "actionType": "like_playlist" }))
    }

    pub async fn unlike_playlist(&self, sc_user_id: &str, playlist_urn: &str) -> AppResult<Value> {
        self.sync_queue
            .clear_wanted(
                LIKES_PLAYLISTS,
                sc_user_id,
                playlist_urn,
                "unlike_playlist",
                playlist_urn,
            )
            .await?;
        Ok(json!({ "status": "queued", "actionType": "unlike_playlist" }))
    }

    pub async fn is_playlist_liked(
        &self,
        sc_user_id: &str,
        playlist_urn: &str,
    ) -> AppResult<Value> {
        let uid_variants = crate::common::sc_ids::user_id_variants(sc_user_id);
        let exists = sqlx::query_file_scalar!(
            "queries/likes/service/is_playlist_liked.sql",
            &uid_variants,
            playlist_urn
        )
        .fetch_one(&self.pg)
        .await?;
        Ok(json!({ "liked": exists }))
    }
}
