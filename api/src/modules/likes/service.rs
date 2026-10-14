use std::sync::Arc;

use serde_json::{Value, json};
use sqlx::{PgConnection, PgPool};

use super::order::{self, LikesOrderRequest};
use crate::common::sc_ids::{EntityKind, EntityRef, require_ref};
use crate::error::AppResult;
use crate::modules::events::EventsService;
use crate::modules::sync_queue::SyncQueueService;
use crate::modules::sync_queue::mirror::{LIKES_PLAYLISTS, LIKES_TRACKS};
use crate::modules::tracks::TrackPriority;

pub struct LikesService {
    pg: PgPool,
    sync_queue: Arc<SyncQueueService>,
    events: Arc<EventsService>,
}

impl LikesService {
    pub fn new(
        pg: PgPool,
        sync_queue: Arc<SyncQueueService>,
        events: Arc<EventsService>,
    ) -> Arc<Self> {
        Arc::new(Self {
            pg,
            sync_queue,
            events,
        })
    }

    pub async fn like_track(&self, sc_user_id: &str, track_urn: &str) -> AppResult<Value> {
        let track = require_ref(EntityKind::Track, track_urn)?;
        let sc_track_id = &track.sc_id();
        crate::modules::cold_refresh::entity::want_track(
            &self.pg,
            sc_track_id,
            TrackPriority::Like,
        )
        .await?;
        let mut transaction = self.pg.begin().await?;
        let liked = like_track_on(&mut transaction, &self.sync_queue, sc_user_id, &track).await?;
        transaction.commit().await?;
        if liked {
            self.events
                .record(sc_user_id, sc_track_id, "like", None)
                .await?;
        }
        Ok(json!({ "status": "queued", "actionType": "like_track" }))
    }

    pub async fn unlike_track(&self, sc_user_id: &str, track_urn: &str) -> AppResult<Value> {
        let track = require_ref(EntityKind::Track, track_urn)?;
        let (sc_track_id, track_urn) = (&track.sc_id(), &track.urn());
        self.sync_queue
            .clear_wanted(
                LIKES_TRACKS,
                sc_user_id,
                sc_track_id,
                "unlike_track",
                track_urn,
            )
            .await?;
        order::forget(&self.pg, sc_user_id, sc_track_id).await?;
        crate::modules::superlikes::service::forget(&self.pg, sc_user_id, sc_track_id).await?;
        Ok(json!({ "status": "queued", "actionType": "unlike_track" }))
    }

    pub async fn order_tracks(
        &self,
        sc_user_id: &str,
        request: LikesOrderRequest,
    ) -> AppResult<()> {
        order::apply(&self.pg, sc_user_id, request).await
    }

    pub async fn like_playlist(&self, sc_user_id: &str, playlist_urn: &str) -> AppResult<Value> {
        let playlist_urn = &require_ref(EntityKind::Playlist, playlist_urn)?.urn();
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
        let playlist_urn = &require_ref(EntityKind::Playlist, playlist_urn)?.urn();
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
        let playlist_urn = &require_ref(EntityKind::Playlist, playlist_urn)?.urn();
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

pub async fn like_track_on(
    connection: &mut PgConnection,
    sync_queue: &SyncQueueService,
    sc_user_id: &str,
    track: &EntityRef,
) -> AppResult<bool> {
    sync_queue
        .set_wanted_on(
            connection,
            LIKES_TRACKS,
            sc_user_id,
            &track.sc_id(),
            "like_track",
            &track.urn(),
        )
        .await
}
