use crate::queue::{JobError, JobRepository, JobResult, LeasedJob, NewJob};
use backend_contracts::{
    CatalogEntity, CatalogRefreshPayload, IndexTrackPayload, JobKind, Versioned,
};
use catalog_ingest::{ScTrackFields, TrackPriority};
use chrono::Utc;
use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;

pub(super) struct CatalogWriter {
    pool: PgPool,
    queue: JobRepository,
    max_track_duration_ms: i32,
}

impl CatalogWriter {
    pub(super) async fn is_deleted(&self, payload: &CatalogRefreshPayload) -> JobResult<bool> {
        let entity = match payload.entity {
            CatalogEntity::Track => "track",
            CatalogEntity::Playlist => "playlist",
            _ => return Ok(false),
        };
        sqlx::query_file_scalar!(
            "queries/catalog_refresh/is_deleted.sql",
            entity,
            &payload.sc_id
        )
        .fetch_one(&self.pool)
        .await
        .map_err(JobError::retryable)
    }

    pub(super) async fn begin_observation(&self) -> JobResult<catalog_ingest::Observation> {
        catalog_ingest::Observation::begin(&self.pool)
            .await
            .map_err(JobError::retryable)
    }

    pub(super) fn new(pool: PgPool, max_track_duration_ms: i32) -> Self {
        Self {
            queue: JobRepository::new(pool.clone(), "catalog-refresh".to_owned()),
            pool,
            max_track_duration_ms,
        }
    }

    pub(super) async fn persist(
        &self,
        job: &LeasedJob,
        payload: &CatalogRefreshPayload,
        value: &Value,
        observation: catalog_ingest::Observation,
    ) -> JobResult {
        let mut transaction = self.pool.begin().await.map_err(JobError::retryable)?;
        let fence = sqlx::query_file_scalar!(
            "queries/catalog_refresh/fence.sql",
            job.id,
            job.lease_id,
            job.generation
        )
        .fetch_optional(&mut *transaction)
        .await
        .map_err(JobError::retryable)?;
        if fence.is_none() {
            return Ok(());
        }
        self.persist_in(
            &mut transaction,
            payload,
            value,
            TrackPriority::Discovery,
            observation,
        )
        .await?;
        transaction.commit().await.map_err(JobError::retryable)
    }

    pub(super) async fn persist_in(
        &self,
        connection: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        payload: &CatalogRefreshPayload,
        value: &Value,
        priority: TrackPriority,
        observation: catalog_ingest::Observation,
    ) -> JobResult {
        match payload.entity {
            CatalogEntity::Track => {
                let fields = ScTrackFields::from_sc(value).ok_or_else(|| {
                    JobError::retryable(anyhow::anyhow!("invalid catalog track payload"))
                })?;
                let result = catalog_ingest::upsert_track_in(
                    connection,
                    &fields,
                    priority,
                    priority,
                    observation,
                )
                .await
                .map_err(JobError::retryable)?;
                if result.metadata_applied && fields.duration_ms > self.max_track_duration_ms {
                    sqlx::query_file!(
                        "../api/queries/tracks/mark_too_long.sql",
                        &fields.sc_track_id
                    )
                    .execute(&mut **connection)
                    .await
                    .map_err(JobError::retryable)?;
                } else if result.was_new {
                    let downstream = NewJob {
                        id: Uuid::now_v7(),
                        kind: JobKind::IndexTrack,
                        dedup_key: Some(fields.sc_track_id.clone()),
                        payload: serde_json::to_value(Versioned::V1(IndexTrackPayload {
                            sc_track_id: fields.sc_track_id.clone(),
                        }))
                        .map_err(JobError::permanent)?,
                        priority: 5,
                        max_attempts: 8,
                        available_at: Utc::now(),
                    };
                    self.queue
                        .enqueue_in_if_absent(connection, &downstream)
                        .await
                        .map_err(JobError::retryable)?;
                    super::lyrics::wake::enqueue_in(&self.queue, connection, &fields.sc_track_id)
                        .await?;
                }
            }
            CatalogEntity::Playlist => {
                catalog_ingest::upsert_playlist_in(connection, value, observation)
                    .await
                    .map_err(JobError::retryable)?;
            }
            CatalogEntity::User => {
                catalog_ingest::upsert_user_in(connection, value, observation)
                    .await
                    .map_err(JobError::retryable)?;
            }
            CatalogEntity::Profile => {
                catalog_ingest::upsert_profile_in(connection, &payload.sc_id, value, observation)
                    .await
                    .map_err(JobError::retryable)?;
            }
            CatalogEntity::WebProfiles => {
                super::catalog_web_profiles::validate(value)?;
                sqlx::query_file!(
                    "queries/catalog_refresh/upsert_web_profiles.sql",
                    &payload.sc_id,
                    value,
                    observation.sequence()
                )
                .execute(&mut **connection)
                .await
                .map_err(JobError::retryable)?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "catalog_refresh_writer_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "catalog_web_profiles_writer_tests.rs"]
mod web_profiles_tests;
