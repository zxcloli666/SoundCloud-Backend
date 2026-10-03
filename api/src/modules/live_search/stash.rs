use std::future::Future;
use std::sync::Arc;

use catalog_ingest::Observation;
use serde_json::Value;
use sqlx::PgPool;
use tracing::debug;

use super::query::ADOPT_CAP;
use super::store::LiveStore;
use crate::cache::CacheService;
use crate::error::AppResult;
use crate::modules::indexing::IndexingService;
use crate::modules::playlists::PlaylistRepository;
use crate::modules::tracks::TrackPriority;
use crate::modules::users::UserRepository;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Adoption {
    Unseen,
    Adopted,
    Failed,
}

impl Adoption {
    pub fn was_seen(self) -> bool {
        self != Self::Unseen
    }
}

pub struct LiveStash {
    store: LiveStore,
}

impl LiveStash {
    pub fn new(cache: Arc<CacheService>) -> Arc<Self> {
        Arc::new(Self {
            store: LiveStore::new(cache),
        })
    }

    pub async fn adopt_track(&self, indexing: &Arc<IndexingService>, urn: &str) -> Adoption {
        self.adopt("track", urn, |item| async move {
            indexing
                .ingest_track_from_sc(&item, TrackPriority::Discovery, Observation::UNVERIFIED)
                .await
        })
        .await
    }

    pub async fn adopt_user(&self, pg: &PgPool, urn: &str) -> Adoption {
        self.adopt("user", urn, |item| async move {
            UserRepository::new(pg.clone())
                .upsert_from_sc(&item, Observation::UNVERIFIED)
                .await
        })
        .await
    }

    pub async fn adopt_playlist(&self, pg: &PgPool, urn: &str) -> Adoption {
        self.adopt("playlist", urn, |item| async move {
            PlaylistRepository::new(pg.clone())
                .upsert_from_sc(&item, Observation::UNVERIFIED)
                .await
        })
        .await
    }

    async fn adopt<T, F, Fut>(&self, entity: &'static str, urn: &str, upsert: F) -> Adoption
    where
        F: FnOnce(Value) -> Fut,
        Fut: Future<Output = AppResult<T>>,
    {
        let Some(item) = self.store.item(urn).await else {
            crate::metrics::record_live_adopt(entity, "miss");
            return Adoption::Unseen;
        };
        match tokio::time::timeout(ADOPT_CAP, upsert(item)).await {
            Ok(Ok(_)) => {
                crate::metrics::record_live_adopt(entity, "adopted");
                Adoption::Adopted
            }
            Ok(Err(error)) => {
                debug!(entity, urn, %error, "a live search hit could not be adopted");
                crate::metrics::record_live_adopt(entity, "failed");
                Adoption::Failed
            }
            Err(_) => {
                debug!(entity, urn, "adopting a live search hit timed out");
                crate::metrics::record_live_adopt(entity, "failed");
                Adoption::Failed
            }
        }
    }
}
