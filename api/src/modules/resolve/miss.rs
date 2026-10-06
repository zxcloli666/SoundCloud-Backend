use std::sync::Arc;
use std::time::Duration;

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
const ADOPT_TRACKS_DEADLINE: Duration = Duration::from_secs(8);
const ADOPT_ONE_DEADLINE: Duration = Duration::from_secs(5);

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
        let fetched: Vec<Fetched> = futures::stream::iter(missing)
            .map(|id| async move { self.fetch(session, CatalogEntity::Track, &id).await.ok() })
            .buffer_unordered(ADOPT_CONCURRENCY)
            .take_until(tokio::time::sleep(ADOPT_TRACKS_DEADLINE))
            .filter_map(futures::future::ready)
            .collect()
            .await;
        futures::stream::iter(fetched)
            .map(|fetched| self.store(fetched, TrackPriority::Playlist))
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
        match self.fetch(session, entity, id).await {
            Ok(fetched) => self.store(fetched, priority).await,
            Err(outcome) => outcome,
        }
    }

    async fn fetch(
        &self,
        session: Uuid,
        entity: CatalogEntity,
        id: &str,
    ) -> Result<Fetched, Adopted> {
        if id.is_empty() || !id.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err(Adopted::Unavailable);
        }
        if self
            .admission
            .check_session(Endpoint::CatalogMiss, session)
            .await
            .is_err()
        {
            return Err(Adopted::Unavailable);
        }
        let Ok(read) =
            tokio::time::timeout(ADOPT_ONE_DEADLINE, self.read(session, entity, id)).await
        else {
            tracing::debug!(?entity, id, "catalog miss fetch ran out of time");
            return Err(Adopted::Unavailable);
        };
        read.map_err(|error| {
            tracing::debug!(%error, ?entity, id, "catalog miss was not fetched");
            outcome_of(&error)
        })
    }

    async fn read(&self, session: Uuid, entity: CatalogEntity, id: &str) -> AppResult<Fetched> {
        let observation = Observation::begin(&self.pg).await?;
        let kind = TokenKind::UserFirst(session);
        let value = match entity {
            CatalogEntity::Track => self.reads.track_by_id(kind, id).await?,
            CatalogEntity::User => self.reads.user_by_id(kind, id).await?,
            CatalogEntity::Playlist => self.reads.playlist_meta(kind, id).await?,
            CatalogEntity::Profile | CatalogEntity::WebProfiles => {
                return Err(AppError::bad_request("Unsupported entity"));
            }
        };
        validate_entity_identity(&value, entity, id)?;
        Ok(Fetched {
            entity,
            value,
            observation,
        })
    }

    async fn store(&self, fetched: Fetched, priority: TrackPriority) -> Adopted {
        match self
            .persist(
                fetched.entity,
                &fetched.value,
                priority,
                fetched.observation,
            )
            .await
        {
            Ok(()) => Adopted::Stored,
            Err(error) => {
                tracing::debug!(%error, entity = ?fetched.entity, "catalog miss was not stored");
                outcome_of(&error)
            }
        }
    }
}

struct Fetched {
    entity: CatalogEntity,
    value: Value,
    observation: Observation,
}

pub(super) fn outcome_of(error: &AppError) -> Adopted {
    match error {
        AppError::ScApi { status: 404, .. } => Adopted::Gone,
        _ => Adopted::Unavailable,
    }
}
