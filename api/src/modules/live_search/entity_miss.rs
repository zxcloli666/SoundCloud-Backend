use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use backend_contracts::CatalogEntity;
use catalog_ingest::Observation;
use sc_transport::EgressHealth;
use serde_json::Value;
use sqlx::PgPool;
use tracing::debug;

use super::gate::{BREAKER_CHANNEL, PAUSE_CHANNEL};
use super::query::identity_of;
use crate::common::admission::{Decision, Endpoint, PublicAdmission};
use crate::common::sc_ids::extract_sc_id;
use crate::common::sc_payload::validate_entity_identity;
use crate::error::AppResult;
use crate::modules::auth::TokenKind;
use crate::modules::indexing::IndexingService;
use crate::modules::playlists::PlaylistRepository;
use crate::modules::tracks::TrackPriority;
use crate::sc::{EGRESS_APP, PgEgressHealth, ScReadService};

pub const READ_CAP: Duration = Duration::from_millis(3000);

pub struct EntityMiss {
    read: Arc<ScReadService>,
    admission: Arc<PublicAdmission>,
    pause: EgressHealth,
    breaker: EgressHealth,
    pg: PgPool,
}

impl EntityMiss {
    pub fn new(read: Arc<ScReadService>, admission: Arc<PublicAdmission>, pg: PgPool) -> Self {
        Self {
            read,
            admission,
            pause: EgressHealth::new(
                PAUSE_CHANNEL,
                EGRESS_APP,
                Some(PgEgressHealth::new(pg.clone())),
            ),
            breaker: EgressHealth::new(
                BREAKER_CHANNEL,
                EGRESS_APP,
                Some(PgEgressHealth::new(pg.clone())),
            ),
            pg,
        }
    }

    pub async fn track(
        &self,
        indexing: &Arc<IndexingService>,
        urn: &str,
        sc_user_id: &str,
    ) -> bool {
        let id = extract_sc_id(urn);
        self.read_through(
            CatalogEntity::Track,
            "track",
            id,
            sc_user_id,
            || self.read.track_by_id(TokenKind::PublicPool, id),
            |item, observation| async move {
                indexing
                    .ingest_track_from_sc(&item, TrackPriority::Discovery, observation)
                    .await
            },
        )
        .await
    }

    pub async fn playlist(&self, urn: &str, sc_user_id: &str) -> bool {
        let id = extract_sc_id(urn);
        self.read_through(
            CatalogEntity::Playlist,
            "playlist",
            id,
            sc_user_id,
            || self.read.playlist_meta(TokenKind::PublicPool, id),
            |item, observation| async move {
                PlaylistRepository::new(self.pg.clone())
                    .upsert_from_sc(&item, observation)
                    .await
                    .map(|_| ())
            },
        )
        .await
    }

    async fn read_through<R, RFut, S, SFut>(
        &self,
        entity: CatalogEntity,
        label: &'static str,
        id: &str,
        sc_user_id: &str,
        read: R,
        store: S,
    ) -> bool
    where
        R: FnOnce() -> RFut,
        RFut: Future<Output = AppResult<Value>>,
        S: FnOnce(Value, Observation) -> SFut,
        SFut: Future<Output = AppResult<()>>,
    {
        if !self.admits(sc_user_id).await {
            crate::metrics::record_entity_miss_read(label, "closed");
            return false;
        }
        let Ok(observation) = Observation::begin(&self.pg).await else {
            crate::metrics::record_entity_miss_read(label, "failed");
            return false;
        };
        let item = match tokio::time::timeout(READ_CAP, read()).await {
            Ok(Ok(item)) => item,
            Ok(Err(error)) => {
                debug!(entity = label, id, %error, "an inline read of a missing entity failed");
                crate::metrics::record_entity_miss_read(label, "failed");
                return false;
            }
            Err(_) => {
                crate::metrics::record_entity_miss_read(label, "timeout");
                return false;
            }
        };
        let stored = match validate_entity_identity(&item, entity, id) {
            Ok(()) => store(item, observation).await,
            Err(error) => Err(error),
        };
        match stored {
            Ok(()) => {
                crate::metrics::record_entity_miss_read(label, "stored");
                true
            }
            Err(error) => {
                debug!(entity = label, id, %error, "an inline read could not be stored");
                crate::metrics::record_entity_miss_read(label, "failed");
                false
            }
        }
    }

    async fn admits(&self, sc_user_id: &str) -> bool {
        self.pause.open_for().await.is_none()
            && self.breaker.open_for().await.is_none()
            && self
                .admission
                .check_identity(Endpoint::LiveEntity, &identity_of(sc_user_id))
                .await
                == Decision::Allowed
    }
}
