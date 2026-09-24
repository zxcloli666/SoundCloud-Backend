mod model;
mod store;

use sqlx::PgPool;
use tracing::info;

use crate::config::SubscriptionsConfig;
use crate::queue::{JobError, JobResult};

use self::model::{Subscription, SubscriptionSnapshot};
use self::store::{SnapshotStore, StoreError, StoredSnapshot};

pub(super) struct SubscriptionSnapshotHandler {
    pool: PgPool,
    store: SnapshotStore,
    max_entries: usize,
}

impl SubscriptionSnapshotHandler {
    pub fn new(pool: PgPool, config: &SubscriptionsConfig) -> Self {
        Self {
            pool,
            store: SnapshotStore::new(config.snapshot_dir.clone(), config.max_file_bytes),
            max_entries: config.max_entries,
        }
    }

    pub async fn bootstrap(&self) -> JobResult {
        self.store
            .verify_writable()
            .await
            .map_err(JobError::permanent)?;
        let status = sqlx::query_file!("queries/subscriptions/bootstrap_status.sql")
            .fetch_one(&self.pool)
            .await
            .map_err(JobError::retryable)?;
        if status.initialized {
            return Ok(());
        }

        let snapshot = if status.has_subscriptions {
            None
        } else {
            match self.store.load().await.map_err(store_job_error)? {
                StoredSnapshot::Missing => None,
                StoredSnapshot::Found(bytes) => Some(
                    SubscriptionSnapshot::decode(&bytes, self.max_entries)
                        .map_err(JobError::permanent)?,
                ),
            }
        };

        let outcome = self.finish_bootstrap(snapshot).await?;
        match outcome {
            BootstrapOutcome::AlreadyInitialized => {}
            BootstrapOutcome::DatabasePopulated => {
                info!("subscription snapshot restore skipped because the database is populated");
            }
            BootstrapOutcome::StartedFresh => {
                info!("subscription snapshot bootstrap started without a saved snapshot");
            }
            BootstrapOutcome::Restored(count) => {
                info!(count, "subscriptions restored from snapshot");
            }
        }
        Ok(())
    }

    pub async fn export(&self) -> JobResult {
        let query_limit = self
            .max_entries
            .checked_add(1)
            .and_then(|value| i64::try_from(value).ok())
            .ok_or_else(|| {
                JobError::permanent(anyhow::anyhow!("snapshot entry limit is invalid"))
            })?;
        let entries = sqlx::query_file_as!(
            Subscription,
            "queries/subscriptions/export.sql",
            query_limit
        )
        .fetch_all(&self.pool)
        .await
        .map_err(JobError::retryable)?;
        let snapshot = SubscriptionSnapshot::from_entries(entries, self.max_entries)
            .map_err(JobError::permanent)?;
        let count = snapshot.len();
        let bytes = snapshot
            .encode(self.store.max_bytes())
            .map_err(JobError::permanent)?;
        self.store.replace(&bytes).await.map_err(store_job_error)?;
        info!(count, bytes = bytes.len(), "subscription snapshot exported");
        Ok(())
    }

    async fn finish_bootstrap(
        &self,
        snapshot: Option<SubscriptionSnapshot>,
    ) -> JobResult<BootstrapOutcome> {
        let mut transaction = self.pool.begin().await.map_err(JobError::retryable)?;
        sqlx::query_file!("queries/subscriptions/lock_bootstrap_state.sql")
            .execute(&mut *transaction)
            .await
            .map_err(JobError::retryable)?;
        sqlx::query_file!("queries/subscriptions/lock_table.sql")
            .execute(&mut *transaction)
            .await
            .map_err(JobError::retryable)?;
        let status = sqlx::query_file!("queries/subscriptions/bootstrap_status.sql")
            .fetch_one(&mut *transaction)
            .await
            .map_err(JobError::retryable)?;

        if status.initialized {
            transaction.commit().await.map_err(JobError::retryable)?;
            return Ok(BootstrapOutcome::AlreadyInitialized);
        }
        if status.has_subscriptions {
            mark_initialized(&mut transaction).await?;
            transaction.commit().await.map_err(JobError::retryable)?;
            return Ok(BootstrapOutcome::DatabasePopulated);
        }

        let outcome = match snapshot {
            Some(snapshot) => restore_snapshot(&mut transaction, snapshot).await?,
            None => BootstrapOutcome::StartedFresh,
        };
        mark_initialized(&mut transaction).await?;
        transaction.commit().await.map_err(JobError::retryable)?;
        Ok(outcome)
    }
}

async fn restore_snapshot(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    snapshot: SubscriptionSnapshot,
) -> JobResult<BootstrapOutcome> {
    let count = snapshot.len();
    if count == 0 {
        return Ok(BootstrapOutcome::Restored(0));
    }
    let (user_urns, exp_dates) = snapshot.into_columns();
    sqlx::query_file!("queries/subscriptions/restore.sql", &user_urns, &exp_dates)
        .execute(&mut **transaction)
        .await
        .map_err(JobError::retryable)?;
    Ok(BootstrapOutcome::Restored(count))
}

async fn mark_initialized(transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>) -> JobResult {
    sqlx::query_file!("queries/subscriptions/mark_initialized.sql")
        .execute(&mut **transaction)
        .await
        .map_err(JobError::retryable)?;
    Ok(())
}

fn store_job_error(error: StoreError) -> JobError {
    if error.is_permanent() {
        JobError::permanent(error)
    } else {
        JobError::retryable(error)
    }
}

enum BootstrapOutcome {
    AlreadyInitialized,
    DatabasePopulated,
    StartedFresh,
    Restored(usize),
}

#[cfg(test)]
mod tests;
