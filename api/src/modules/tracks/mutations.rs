use std::sync::Arc;

use catalog_ingest::TrackUpdate;
use serde_json::{Value, json};
use sqlx::PgPool;

use crate::common::sc_ids::extract_sc_id;
use crate::error::{AppError, AppResult};
use crate::modules::sync_queue::SyncQueueService;

pub(super) struct TrackMutations {
    pg: PgPool,
    queue: Arc<SyncQueueService>,
}

impl TrackMutations {
    pub(super) fn new(pg: PgPool, queue: Arc<SyncQueueService>) -> Self {
        Self { pg, queue }
    }

    async fn begin(&self, target: &str) -> AppResult<sqlx::Transaction<'_, sqlx::Postgres>> {
        let mut tx = self.pg.begin().await?;
        sqlx::query_file_scalar!("queries/tracks/service/lock_mutation.sql", target)
            .fetch_one(&mut *tx)
            .await?;
        Ok(tx)
    }

    pub(super) async fn update(&self, user: &str, target: &str, body: &Value) -> AppResult<Value> {
        crate::db::retry_conflicts(|| self.update_once(user, target, body)).await
    }

    async fn update_once(&self, user: &str, target: &str, body: &Value) -> AppResult<Value> {
        let target = target_urn(target)?;
        let update = TrackUpdate::parse(body).map_err(AppError::bad_request)?;
        let observation = catalog_ingest::Observation::begin(&self.pg).await?;
        let mut tx = self.begin(&target).await?;
        let edit = wt::TrackEdit {
            sc_track_id: extract_sc_id(&target).to_owned(),
            uploader: extract_sc_id(user).to_owned(),
            desired: update.desired().clone(),
            observation: observation.sequence(),
            at: wt::now(),
        };
        if wt::call(&mut tx, &edit).await?.is_none() {
            return Err(AppError::not_found("Track not found"));
        }
        self.queue
            .enqueue_on(&mut tx, user, "track_update", &target, Some(update.body()))
            .await?;
        let pending = sqlx::query_file_scalar!(
            "queries/tracks/service/pending_update.sql",
            extract_sc_id(user),
            &target
        )
        .fetch_one(&mut *tx)
        .await?;
        TrackUpdate::parse(&pending).map_err(AppError::bad_request)?;
        tx.commit().await?;
        Ok(json!({"status": "queued", "actionType": "track_update", "targetUrn": target}))
    }

    pub(super) async fn delete(&self, user: &str, target: &str) -> AppResult<Value> {
        crate::db::retry_conflicts(|| self.delete_once(user, target)).await
    }

    async fn delete_once(&self, user: &str, target: &str) -> AppResult<Value> {
        let target = target_urn(target)?;
        let observation = catalog_ingest::Observation::begin(&self.pg).await?;
        let mut tx = self.begin(&target).await?;
        let delete = wt::TrackDelete {
            sc_track_id: extract_sc_id(&target).to_owned(),
            uploader: extract_sc_id(user).to_owned(),
            observation: observation.sequence(),
            at: wt::now(),
        };
        if wt::call(&mut tx, &delete).await?.is_none() {
            return Err(AppError::not_found("Track not found"));
        }
        let owned = wt::OwnedDelete::Track {
            variants: crate::common::sc_ids::user_id_variants(user),
            sc_track_id: extract_sc_id(&target).to_owned(),
        };
        wt::call(&mut tx, &owned).await?;
        sqlx::query_file!(
            "queries/tracks/service/cancel_updates.sql",
            extract_sc_id(user),
            &target
        )
        .execute(&mut *tx)
        .await?;
        self.queue
            .enqueue_on(&mut tx, user, "track_delete", &target, None)
            .await?;
        tx.commit().await?;
        Ok(json!({"status": "queued", "actionType": "track_delete", "targetUrn": target}))
    }
}

pub(crate) fn target_urn(target: &str) -> AppResult<String> {
    crate::common::sc_ids::require_ref(crate::common::sc_ids::EntityKind::Track, target)
        .map(crate::common::sc_ids::EntityRef::urn)
}

#[cfg(test)]
#[path = "mutation_tests.rs"]
mod tests;
