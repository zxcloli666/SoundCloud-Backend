use std::sync::Arc;

use backend_contracts::CatalogEntity;
use catalog_ingest::{Observation, TrackPriority};
use futures::StreamExt;
use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;

use crate::common::admission::{Endpoint, PublicAdmission};
use crate::common::sc_payload::validate_entity_identity;
use crate::error::{AppError, AppResult};
use crate::modules::auth::TokenKind;
use crate::modules::indexing::IndexingService;
use crate::modules::playlists::PlaylistRepository;
use crate::modules::users::UserRepository;
use crate::sc::ScReadService;

const ADOPT_CONCURRENCY: usize = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Adopted {
    Stored,
    Gone,
    Unavailable,
}

pub struct CatalogMiss {
    pg: PgPool,
    reads: Arc<ScReadService>,
    indexing: Arc<IndexingService>,
    admission: Arc<PublicAdmission>,
}

impl CatalogMiss {
    pub fn new(
        pg: PgPool,
        reads: Arc<ScReadService>,
        indexing: Arc<IndexingService>,
        admission: Arc<PublicAdmission>,
    ) -> Arc<Self> {
        Arc::new(Self {
            pg,
            reads,
            indexing,
            admission,
        })
    }

    pub async fn track(
        &self,
        session: Uuid,
        sc_track_id: &str,
        priority: TrackPriority,
    ) -> Adopted {
        self.adopt(session, CatalogEntity::Track, sc_track_id, priority)
            .await
    }

    pub async fn user(&self, session: Uuid, sc_user_id: &str) -> Adopted {
        self.adopt(
            session,
            CatalogEntity::User,
            sc_user_id,
            TrackPriority::Discovery,
        )
        .await
    }

    pub async fn playlist(&self, session: Uuid, sc_playlist_id: &str) -> Adopted {
        self.adopt(
            session,
            CatalogEntity::Playlist,
            sc_playlist_id,
            TrackPriority::Discovery,
        )
        .await
    }

    pub async fn tracks(&self, session: Uuid, ids: &[String]) -> AppResult<()> {
        if ids.is_empty() {
            return Ok(());
        }
        let missing = sqlx::query_file_scalar!("queries/playlists/missing_catalog_tracks.sql", ids)
            .fetch_all(&self.pg)
            .await?;
        futures::stream::iter(missing)
            .map(|id| async move { self.track(session, &id, TrackPriority::Playlist).await })
            .buffer_unordered(ADOPT_CONCURRENCY)
            .collect::<Vec<_>>()
            .await;
        Ok(())
    }

    pub async fn persist(
        &self,
        entity: CatalogEntity,
        value: &Value,
        priority: TrackPriority,
        observation: Observation,
    ) -> AppResult<()> {
        match entity {
            CatalogEntity::Track => {
                self.indexing
                    .ingest_track_from_sc(value, priority, observation)
                    .await
            }
            CatalogEntity::User => {
                UserRepository::new(self.pg.clone())
                    .upsert_from_sc(value, observation)
                    .await?;
                Ok(())
            }
            CatalogEntity::Playlist => {
                PlaylistRepository::new(self.pg.clone())
                    .upsert_from_sc(value, observation)
                    .await?;
                Ok(())
            }
            CatalogEntity::Profile | CatalogEntity::WebProfiles => {
                Err(AppError::bad_request("Unsupported entity"))
            }
        }
    }

    async fn adopt(
        &self,
        session: Uuid,
        entity: CatalogEntity,
        id: &str,
        priority: TrackPriority,
    ) -> Adopted {
        if id.is_empty() || !id.bytes().all(|byte| byte.is_ascii_digit()) {
            return Adopted::Unavailable;
        }
        if self
            .admission
            .check_session(Endpoint::CatalogMiss, session)
            .await
            .is_err()
        {
            return Adopted::Unavailable;
        }
        match self.fetch_and_store(session, entity, id, priority).await {
            Ok(()) => Adopted::Stored,
            Err(error) => {
                tracing::debug!(%error, ?entity, id, "catalog miss was not adopted");
                outcome_of(&error)
            }
        }
    }

    async fn fetch_and_store(
        &self,
        session: Uuid,
        entity: CatalogEntity,
        id: &str,
        priority: TrackPriority,
    ) -> AppResult<()> {
        let observation = Observation::begin(&self.pg).await?;
        let kind = TokenKind::UserFirst(session);
        let fetched = match entity {
            CatalogEntity::Track => self.reads.track_by_id(kind, id).await?,
            CatalogEntity::User => self.reads.user_by_id(kind, id).await?,
            CatalogEntity::Playlist => self.reads.playlist_meta(kind, id).await?,
            CatalogEntity::Profile | CatalogEntity::WebProfiles => {
                return Err(AppError::bad_request("Unsupported entity"));
            }
        };
        validate_entity_identity(&fetched, entity, id)?;
        self.persist(entity, &fetched, priority, observation).await
    }
}

pub(super) fn outcome_of(error: &AppError) -> Adopted {
    match error {
        AppError::ScApi { status: 404, .. } => Adopted::Gone,
        _ => Adopted::Unavailable,
    }
}
