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
        let target = target_urn(target)?;
        let update = TrackUpdate::parse(body).map_err(AppError::bad_request)?;
        let mut tx = self.begin(&target).await?;
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
        let changed = sqlx::query_file_scalar!(
            "queries/tracks/service/apply_update.sql",
            extract_sc_id(&target),
            extract_sc_id(user),
            update.desired()
        )
        .fetch_optional(&mut *tx)
        .await?;
        if changed.is_none() {
            return Err(AppError::not_found("Track not found"));
        }
        tx.commit().await?;
        Ok(json!({"status": "queued", "actionType": "track_update", "targetUrn": target}))
    }

    pub(super) async fn delete(&self, user: &str, target: &str) -> AppResult<Value> {
        let target = target_urn(target)?;
        let mut tx = self.begin(&target).await?;
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
        let changed = sqlx::query_file_scalar!(
            "queries/tracks/service/apply_delete.sql",
            extract_sc_id(&target),
            extract_sc_id(user)
        )
        .fetch_optional(&mut *tx)
        .await?;
        if changed.is_none() {
            return Err(AppError::not_found("Track not found"));
        }
        sqlx::query_file!(
            "queries/tracks/service/delete_owned.sql",
            &crate::common::sc_ids::user_id_variants(user),
            extract_sc_id(&target)
        )
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(json!({"status": "queued", "actionType": "track_delete", "targetUrn": target}))
    }
}

pub(super) fn target_urn(target: &str) -> AppResult<String> {
    let id = extract_sc_id(target);
    let payload = backend_contracts::CatalogRefreshPayload {
        entity: backend_contracts::CatalogEntity::Track,
        sc_id: id.to_owned(),
        owner_id: None,
    };
    let canonical = payload.entity.urn(id);
    if !payload.is_valid() || (target != id && target != canonical) {
        return Err(AppError::bad_request("invalid track identifier"));
    }
    Ok(canonical)
}

#[cfg(test)]
#[path = "mutation_tests.rs"]
mod tests;
