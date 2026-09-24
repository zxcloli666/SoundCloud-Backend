use std::time::Duration;

use serde_json::Value;
use sqlx::PgPool;

use super::actions::{self, ActionError};
use super::model::{ClaimedMutation, LockedMutation};

pub struct SyncQueueRepository {
    pool: PgPool,
    queue: crate::queue::JobRepository,
    lease_duration: Duration,
}

#[derive(Debug, thiserror::Error)]
pub enum FinalizeError {
    #[error("sync queue database operation failed: {0}")]
    Database(#[from] sqlx::Error),

    #[error(transparent)]
    Action(#[from] ActionError),

    #[error(transparent)]
    Queue(#[from] crate::queue::QueueError),
}

impl SyncQueueRepository {
    pub fn new(pool: PgPool, lease_duration: Duration) -> Self {
        Self {
            queue: crate::queue::JobRepository::new(pool.clone(), "sync-confirmation".into()),
            pool,
            lease_duration,
        }
    }

    pub async fn claim(&self, limit: i64) -> Result<Vec<ClaimedMutation>, sqlx::Error> {
        sqlx::query_file_as!(
            ClaimedMutation,
            "queries/sync_queue/claim.sql",
            self.lease_milliseconds(),
            limit
        )
        .fetch_all(&self.pool)
        .await
    }

    pub async fn force_due(&self) -> Result<u64, sqlx::Error> {
        let result = sqlx::query_file!(
            "queries/sync_queue/force_due.sql",
            self.lease_milliseconds()
        )
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected())
    }

    pub async fn record_remote_success(
        &self,
        mutation: &ClaimedMutation,
        remote_result: &Value,
    ) -> Result<bool, sqlx::Error> {
        let result = sqlx::query_file!(
            "queries/sync_queue/record_remote_success.sql",
            mutation.id,
            mutation.lease_id,
            mutation.lease_generation,
            mutation.generation,
            remote_result
        )
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() == 1)
    }

    pub async fn record_remote_attempt(
        &self,
        mutation: &ClaimedMutation,
    ) -> Result<bool, sqlx::Error> {
        let result = sqlx::query_file!(
            "queries/sync_queue/record_remote_attempt.sql",
            mutation.id,
            mutation.lease_id,
            mutation.lease_generation,
            mutation.generation
        )
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() == 1)
    }

    pub async fn finalize(&self, mutation: &ClaimedMutation) -> Result<(), FinalizeError> {
        let mut transaction = self.pool.begin().await?;
        let locked = sqlx::query_file_as!(
            LockedMutation,
            "queries/sync_queue/lock_for_finalize.sql",
            mutation.id
        )
        .fetch_optional(&mut *transaction)
        .await?;
        let Some(locked) = locked else {
            transaction.commit().await?;
            return Ok(());
        };
        if !locked.is_ready_to_finalize(mutation) {
            if locked.is_owned_by(mutation) {
                sqlx::query_file!(
                    "queries/sync_queue/release_lease.sql",
                    mutation.id,
                    mutation.lease_id,
                    mutation.lease_generation
                )
                .execute(&mut *transaction)
                .await?;
            }
            transaction.commit().await?;
            return Ok(());
        }
        let remote_result = locked
            .remote_result
            .as_ref()
            .ok_or(ActionError::InvalidRemoteResult("remote result is missing"))?;
        self.enqueue_confirmation(&mut transaction, mutation)
            .await?;
        actions::apply_local(&mut transaction, mutation, remote_result).await?;
        sqlx::query_file!(
            "queries/sync_queue/delete_completed.sql",
            mutation.id,
            mutation.lease_id,
            mutation.lease_generation
        )
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;
        Ok(())
    }

    async fn enqueue_confirmation(
        &self,
        transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        mutation: &ClaimedMutation,
    ) -> Result<(), crate::queue::QueueError> {
        use backend_contracts::{CatalogEntity, CatalogRefreshPayload, JobKind, Versioned};
        let entity = match mutation.action_type.as_str() {
            "track_update" => CatalogEntity::Track,
            "playlist_update" => CatalogEntity::Playlist,
            _ => return Ok(()),
        };
        let payload = CatalogRefreshPayload {
            entity,
            sc_id: catalog_ingest::extract_sc_id(&mutation.target_urn).to_owned(),
            owner_id: Some(catalog_ingest::extract_sc_id(&mutation.user_id).to_owned()),
        };
        let job = crate::queue::NewJob {
            id: uuid::Uuid::now_v7(),
            kind: JobKind::CatalogRefresh,
            dedup_key: Some(payload.dedup_key()),
            payload: serde_json::json!(Versioned::V1(payload)),
            priority: 10,
            max_attempts: 8,
            available_at: chrono::Utc::now(),
        };
        self.queue.enqueue_in(transaction, &job).await
    }

    pub async fn release(&self, mutation: &ClaimedMutation) -> Result<(), sqlx::Error> {
        sqlx::query_file!(
            "queries/sync_queue/release_lease.sql",
            mutation.id,
            mutation.lease_id,
            mutation.lease_generation
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn retry(
        &self,
        mutation: &ClaimedMutation,
        message: &str,
        retry_after_seconds: i64,
    ) -> Result<bool, sqlx::Error> {
        let result = sqlx::query_file!(
            "queries/sync_queue/retry.sql",
            mutation.id,
            mutation.lease_id,
            mutation.lease_generation,
            bounded_message(message),
            retry_after_seconds.max(1)
        )
        .execute(&self.pool)
        .await?;
        if result.rows_affected() == 0 {
            self.release(mutation).await?;
            return Ok(false);
        }
        Ok(true)
    }

    pub async fn postpone(
        &self,
        mutation: &ClaimedMutation,
        message: &str,
        retry_after_seconds: i64,
    ) -> Result<bool, sqlx::Error> {
        let result = sqlx::query_file!(
            "queries/sync_queue/postpone.sql",
            mutation.id,
            mutation.lease_id,
            mutation.lease_generation,
            bounded_message(message),
            retry_after_seconds.max(1)
        )
        .execute(&self.pool)
        .await?;
        if result.rows_affected() == 0 {
            self.release(mutation).await?;
            return Ok(false);
        }
        Ok(true)
    }

    pub async fn postpone_unattempted(
        &self,
        mutation: &ClaimedMutation,
        message: &str,
        retry_after_seconds: i64,
    ) -> Result<bool, sqlx::Error> {
        let result = sqlx::query_file!(
            "queries/sync_queue/postpone_unattempted.sql",
            mutation.id,
            mutation.lease_id,
            mutation.lease_generation,
            bounded_message(message),
            retry_after_seconds.max(1)
        )
        .execute(&self.pool)
        .await?;
        if result.rows_affected() == 0 {
            self.release(mutation).await?;
            return Ok(false);
        }
        Ok(true)
    }

    pub async fn park(
        &self,
        mutation: &ClaimedMutation,
        retry_count: i32,
        message: &str,
    ) -> Result<bool, sqlx::Error> {
        let result = sqlx::query_file!(
            "queries/sync_queue/park.sql",
            mutation.id,
            mutation.lease_id,
            mutation.lease_generation,
            retry_count,
            bounded_message(message)
        )
        .execute(&self.pool)
        .await?;
        if result.rows_affected() == 0 {
            self.release(mutation).await?;
            return Ok(false);
        }
        Ok(true)
    }

    fn lease_milliseconds(&self) -> i64 {
        i64::try_from(self.lease_duration.as_millis()).unwrap_or(i64::MAX)
    }
}

fn bounded_message(message: &str) -> String {
    message.chars().take(500).collect()
}
