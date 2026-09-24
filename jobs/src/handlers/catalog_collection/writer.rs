use super::{mirror, page, state};
use crate::handlers::catalog_refresh_writer::CatalogWriter;
use crate::queue::{JobError, JobResult, LeasedJob};
use backend_contracts::{
    CatalogCollection, CatalogCollectionPayload, CatalogRefreshPayload, CollectionItem,
};
use catalog_ingest::TrackPriority;
use sqlx::PgPool;

pub(super) struct CollectionWriter {
    pool: PgPool,
    entities: CatalogWriter,
}

impl CollectionWriter {
    pub(super) fn new(pool: PgPool, max_track_duration_ms: i32) -> Self {
        Self {
            entities: CatalogWriter::new(pool.clone(), max_track_duration_ms),
            pool,
        }
    }

    pub(super) async fn persist(
        &self,
        job: &LeasedJob,
        payload: &CatalogCollectionPayload,
        snapshot: &state::Snapshot,
        page: page::Page,
        observation: catalog_ingest::Observation,
    ) -> JobResult {
        let mut tx = self.pool.begin().await.map_err(JobError::retryable)?;
        if !state::fence(&mut tx, job).await? {
            return Ok(());
        }
        let current = state::load(&mut tx, job, payload).await?;
        if current.snapshot_id != snapshot.snapshot_id
            || current.page_count != snapshot.page_count
            || current.complete
        {
            return Ok(());
        }
        let mut keys = Vec::with_capacity(page.items.len());
        let priority = match payload.collection {
            CatalogCollection::LikedTracks | CatalogCollection::OwnedTracks => TrackPriority::Like,
            CatalogCollection::LikedPlaylists | CatalogCollection::OwnedPlaylists => {
                TrackPriority::Playlist
            }
            CatalogCollection::Followings
            | CatalogCollection::Followers
            | CatalogCollection::TrackFavoriters
            | CatalogCollection::TrackReposters
            | CatalogCollection::PlaylistReposters
            | CatalogCollection::TrackComments => TrackPriority::Discovery,
        };
        let comments = payload.collection.item() == CollectionItem::Comment;
        for item in &page.items {
            let entity_json = if comments {
                item.get("user").ok_or_else(|| {
                    JobError::retryable(anyhow::anyhow!("comment author is missing"))
                })?
            } else {
                item
            };
            let urn = entity_json
                .get("urn")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| {
                    JobError::retryable(anyhow::anyhow!("collection entity URN is missing"))
                })?;
            let entity = CatalogRefreshPayload {
                entity: payload.collection.entity(),
                sc_id: catalog_ingest::extract_sc_id(urn).to_owned(),
                owner_id: payload.owner.then(|| payload.subject_id.clone()),
            };
            self.entities
                .persist_in(&mut tx, &entity, entity_json, priority, observation)
                .await?;
            let key = if comments {
                item.get("id")
                    .and_then(serde_json::Value::as_str)
                    .ok_or_else(|| {
                        JobError::retryable(anyhow::anyhow!("comment identity is missing"))
                    })?
                    .to_owned()
            } else if matches!(
                payload.collection,
                CatalogCollection::LikedTracks | CatalogCollection::OwnedTracks
            ) {
                entity.sc_id
            } else {
                urn.to_owned()
            };
            if !keys.contains(&key) {
                keys.push(key);
            }
        }
        if comments {
            mirror::persist_comments(&mut tx, payload, snapshot, &page.items).await?;
        } else {
            mirror::persist(&mut tx, payload, snapshot, &keys).await?;
            mirror::record_like_times(&mut tx, payload, &page.liked_at).await?;
        }
        if payload.collection == CatalogCollection::Followings && payload.owner {
            let subjects: Vec<_> = keys
                .iter()
                .map(|urn| catalog_ingest::extract_sc_id(urn).to_owned())
                .collect();
            sqlx::query_file!(
                "../api/queries/me/service/enqueue_followed_uploads.sql",
                &subjects,
                &payload.subject_id
            )
            .execute(&mut *tx)
            .await
            .map_err(JobError::retryable)?;
        }
        state::advance(&mut tx, payload, snapshot, &keys, page.next.as_deref()).await?;
        if page.next.is_none() {
            mirror::reconcile(&mut tx, payload, snapshot).await?;
        }
        sqlx::query_file!(
            "queries/catalog_collection/reset_attempts.sql",
            job.id,
            job.lease_id,
            job.generation
        )
        .execute(&mut *tx)
        .await
        .map_err(JobError::retryable)?;
        tx.commit().await.map_err(JobError::retryable)
    }
}
