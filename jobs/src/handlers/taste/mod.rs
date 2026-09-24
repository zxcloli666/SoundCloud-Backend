mod artifact;
mod dataset;
mod history;
mod metrics;
mod pooling;
mod result;
mod vectors;

use std::sync::Arc;
use std::time::Duration;

use backend_contracts::pipeline::{
    TASTE_DATA_BUCKET, TASTE_DATASET_VERSION, TRAIN_TASTE, TRAIN_TASTE_STREAM, TasteTrainRequest,
};
use backend_contracts::vector_store::{TRACKS_CLAP, TRACKS_TASTE_DIMENSIONS};
use chrono::NaiveDateTime;
use sqlx::PgPool;
use tokio::time::MissedTickBehavior;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};
use uuid::Uuid;

use crate::bus::{Bus, ObjectStoreError, WorkerQueueSnapshot};
use crate::config::TasteConfig;
use crate::qdrant::{QdrantProvisioner, TRACKS_TASTE};
use crate::queue::{JobError, JobResult};

use self::dataset::Export;
use self::metrics::{ExportSkip, record_export_skipped, record_exported, record_user_vectors};
use self::pooling::Pooling;

pub(crate) use result::TasteResult;

const TICK: Duration = Duration::from_secs(30);
const EXPORT_RETRY: Duration = Duration::from_secs(10 * 60);
const FEATURE_BYTES_PER_TRACK: usize = 6 * 1024;
const EVENT_BYTES_ALLOWANCE: usize = 1024 * 1024 * 1024;
const REFRESH_CATCH_UP: chrono::Duration = chrono::Duration::days(1);
const REFRESH_BATCH: usize = 500;

pub struct TasteHandler {
    pool: PgPool,
    bus: Bus,
    qdrant: QdrantProvisioner,
    config: TasteConfig,
}

struct ActiveVersion {
    version: String,
    collection: String,
    pooling: serde_json::Value,
    refreshed_through: Option<NaiveDateTime>,
}

impl TasteHandler {
    pub fn new(pool: PgPool, bus: Bus, qdrant: QdrantProvisioner, config: TasteConfig) -> Self {
        Self {
            pool,
            bus,
            qdrant,
            config,
        }
    }

    pub async fn run(self: Arc<Self>, cancellation: CancellationToken) -> anyhow::Result<()> {
        metrics::register_series_that_start_at_zero();
        info!(
            dispatch = self.config.dispatch,
            export_interval_s = self.config.export_interval.as_secs(),
            refresh_interval_s = self.config.refresh_interval.as_secs(),
            "taste schedule started"
        );
        let mut ticker = tokio::time::interval(TICK);
        ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                () = cancellation.cancelled() => return Ok(()),
                _ = ticker.tick() => {}
            }
            if self.config.dispatch
                && let Err(error) = self.export_when_due().await
            {
                warn!(%error, "taste export failed; it is retried later");
            }
            if let Err(error) = self.refresh_when_due().await {
                warn!(%error, "taste user vectors were not refreshed");
            }
        }
    }

    async fn export_when_due(&self) -> JobResult {
        let claimed = sqlx::query_file_scalar!(
            "queries/taste/claim_export.sql",
            whole_seconds(self.config.export_interval)
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(JobError::retryable)?;
        if claimed.is_none() {
            return Ok(());
        }
        if let Err(error) = self.dispatch().await {
            self.schedule_export(EXPORT_RETRY).await?;
            return Err(error);
        }
        Ok(())
    }

    async fn schedule_export(&self, after: Duration) -> JobResult {
        sqlx::query_file!("queries/taste/schedule_export.sql", whole_seconds(after))
            .execute(&self.pool)
            .await
            .map_err(JobError::retryable)?;
        Ok(())
    }

    async fn dispatch(&self) -> JobResult {
        if self.training_in_flight().await? {
            info!("taste training is already queued or running; no new export");
            record_export_skipped(ExportSkip::InFlight);
            return Ok(());
        }
        let max_bytes = self.max_object_bytes().await?;
        let dataset = match dataset::export(
            &self.pool,
            &self.qdrant,
            self.config.history_days,
            self.config.min_users,
            max_bytes,
        )
        .await?
        {
            Export::Ready(dataset) => dataset,
            Export::TooFewUsers { users, timed_users } => {
                info!(
                    users,
                    timed_users,
                    minimum = self.config.min_users,
                    "taste training skipped because too few listeners can be tested"
                );
                record_export_skipped(ExportSkip::TooFewUsers);
                return Ok(());
            }
            Export::TooLarge { limit } => {
                warn!(
                    limit,
                    "taste training skipped because its input outgrew the object limit"
                );
                record_export_skipped(ExportSkip::TooLarge);
                return Ok(());
            }
        };

        let object = input_object(Uuid::now_v7());
        let mut file = dataset.open().await.map_err(JobError::retryable)?;
        self.bus
            .put_object_reader(TASTE_DATA_BUCKET, &object, &mut file)
            .await
            .map_err(object_error)?;
        let previous_version = self.active_version().await?.map(|active| active.version);
        let request = TasteTrainRequest {
            object: object.clone(),
            dataset_version: TASTE_DATASET_VERSION,
            dim: TRACKS_TASTE_DIMENSIONS,
            epochs: self.config.epochs,
            batch_size: self.config.batch_size,
            negatives: self.config.negatives,
            seed: self.config.seed,
            previous_version,
        };
        self.bus
            .publish_dedup(TRAIN_TASTE, &request, &train_message_id(&object))
            .await
            .map_err(JobError::retryable)?;
        record_exported();
        info!(
            object,
            users = dataset.users,
            timed_users = dataset.timed_users,
            items = dataset.items,
            bytes = dataset.bytes,
            previous_version = request.previous_version.as_deref(),
            "taste training dispatched"
        );
        Ok(())
    }

    async fn training_in_flight(&self) -> JobResult<bool> {
        let snapshot = self.bus.worker_queue_snapshot().await;
        holds_training(&snapshot).ok_or_else(|| {
            JobError::retryable(anyhow::anyhow!(
                "NATS stream {} is not readable",
                TRAIN_TASTE_STREAM.name
            ))
        })
    }

    async fn max_object_bytes(&self) -> JobResult<usize> {
        if let Some(limit) = self.config.max_object_bytes {
            return Ok(limit);
        }
        let tracks = self
            .qdrant
            .points_count(TRACKS_CLAP)
            .await
            .map_err(JobError::retryable)?;
        Ok(object_limit_for(tracks))
    }

    async fn active_version(&self) -> JobResult<Option<ActiveVersion>> {
        let row = sqlx::query_file!("queries/taste/active_version.sql")
            .fetch_optional(&self.pool)
            .await
            .map_err(JobError::retryable)?;
        Ok(row.map(|row| ActiveVersion {
            version: row.version,
            collection: row.collection,
            pooling: row.pooling,
            refreshed_through: row.refreshed_through,
        }))
    }

    async fn refresh_when_due(&self) -> JobResult {
        let Some(now) = sqlx::query_file_scalar!(
            "queries/taste/claim_refresh.sql",
            whole_seconds(self.config.refresh_interval)
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(JobError::retryable)?
        else {
            return Ok(());
        };
        let Some(active) = self.active_version().await? else {
            return Ok(());
        };
        if let Err(error) = self.serve_alias_of(&active).await {
            warn!(%error, version = active.version, "taste alias could not follow the serving version");
        }
        let pooling = Pooling::from_json(&active.pooling).map_err(JobError::permanent)?;
        let lookback = chrono::Duration::from_std(self.config.refresh_interval)
            .map_err(JobError::permanent)?;
        let (from, until) = refresh_window(active.refreshed_through, now, lookback);
        let users: Vec<String> =
            sqlx::query_file_scalar!("queries/taste/refresh_users.sql", from, until)
                .fetch_all(&self.pool)
                .await
                .map_err(JobError::retryable)?
                .into_iter()
                .filter(|user| is_canonical_user(user))
                .collect();
        let now_unix = now.and_utc().timestamp();
        let mut refreshed = 0usize;
        for chunk in users.chunks(REFRESH_BATCH) {
            let pooled = vectors::pool_users(
                &self.pool,
                &self.qdrant,
                self.config.history_days,
                &pooling,
                &active.collection,
                chunk,
                now_unix,
            )
            .await?;
            vectors::store(&self.pool, &active.version, &pooled).await?;
            refreshed += pooled.len();
        }
        sqlx::query_file!("queries/taste/finish_refresh.sql", active.version, until)
            .execute(&self.pool)
            .await
            .map_err(JobError::retryable)?;
        if refreshed > 0 {
            record_user_vectors(refreshed);
            info!(
                version = active.version,
                users = users.len(),
                refreshed,
                "taste user vectors refreshed"
            );
        }
        Ok(())
    }

    async fn serve_alias_of(&self, active: &ActiveVersion) -> JobResult {
        let target = self
            .qdrant
            .alias_target(TRACKS_TASTE)
            .await
            .map_err(JobError::retryable)?;
        if target.as_deref() == Some(active.collection.as_str()) {
            return Ok(());
        }
        self.qdrant
            .point_alias(TRACKS_TASTE, &active.collection)
            .await
            .map_err(JobError::retryable)?;
        info!(
            version = active.version,
            collection = active.collection,
            before = target.as_deref(),
            "taste alias follows the serving version"
        );
        Ok(())
    }

    async fn remove_object(&self, bucket: &str, name: &str) {
        match self.bus.delete_object(bucket, name).await {
            Ok(()) | Err(ObjectStoreError::NotFound { .. }) => {}
            Err(error) => warn!(bucket, object = name, %error, "taste object could not be removed"),
        }
    }
}

fn refresh_window(
    refreshed_through: Option<NaiveDateTime>,
    now: NaiveDateTime,
    lookback: chrono::Duration,
) -> (NaiveDateTime, NaiveDateTime) {
    let from = refreshed_through.unwrap_or(now - lookback).min(now);
    (from, now.min(from + REFRESH_CATCH_UP))
}

fn is_canonical_user(user: &str) -> bool {
    (1..=18).contains(&user.len()) && user.bytes().all(|byte| byte.is_ascii_digit())
}

fn object_limit_for(tracks: u64) -> usize {
    usize::try_from(tracks)
        .unwrap_or(usize::MAX)
        .saturating_mul(FEATURE_BYTES_PER_TRACK)
        .saturating_add(EVENT_BYTES_ALLOWANCE)
}

fn whole_seconds(duration: Duration) -> i64 {
    i64::try_from(duration.as_secs()).unwrap_or(i64::MAX)
}

fn holds_training(snapshot: &WorkerQueueSnapshot) -> Option<bool> {
    snapshot
        .streams
        .iter()
        .find(|fill| fill.stream == TRAIN_TASTE_STREAM.name)
        .map(|fill| fill.ratio > 0.0)
}

fn input_object(id: Uuid) -> String {
    format!("taste-input-{id}")
}

fn train_message_id(object: &str) -> String {
    format!("train_taste:{object}")
}

fn object_error(error: ObjectStoreError) -> JobError {
    if error.is_permanent() {
        JobError::permanent(error)
    } else {
        JobError::retryable(error)
    }
}

#[cfg(test)]
mod tests {
    use chrono::DateTime;

    use super::*;

    #[test]
    fn a_training_is_deduplicated_by_its_input_object() {
        let object = input_object(Uuid::nil());

        assert_eq!(object, "taste-input-00000000-0000-0000-0000-000000000000");
        assert_eq!(train_message_id(&object), format!("train_taste:{object}"));
    }

    #[test]
    fn the_input_limit_grows_with_the_indexed_catalog() {
        assert_eq!(object_limit_for(0), EVENT_BYTES_ALLOWANCE);
        assert_eq!(
            object_limit_for(100_000),
            100_000 * FEATURE_BYTES_PER_TRACK + EVENT_BYTES_ALLOWANCE
        );
        assert_eq!(object_limit_for(u64::MAX), usize::MAX);
    }

    #[test]
    fn a_refresh_catches_up_one_day_at_a_time_and_never_runs_ahead() {
        let now = DateTime::from_timestamp(10 * 86_400, 0)
            .map(|at| at.naive_utc())
            .unwrap_or_default();
        let five_minutes = chrono::Duration::minutes(5);

        assert_eq!(
            refresh_window(None, now, five_minutes),
            (now - five_minutes, now)
        );
        let far_behind = now - chrono::Duration::days(3);
        assert_eq!(
            refresh_window(Some(far_behind), now, five_minutes),
            (far_behind, far_behind + chrono::Duration::days(1))
        );
        let ahead = now + chrono::Duration::hours(1);
        assert_eq!(refresh_window(Some(ahead), now, five_minutes), (now, now));
    }

    #[test]
    fn only_bare_account_numbers_get_a_vector() {
        assert!(is_canonical_user("12345"));
        assert!(!is_canonical_user(""));
        assert!(!is_canonical_user("soundcloud:users:1"));
        assert!(!is_canonical_user("1234567890123456789"));
    }

    fn snapshot_with(fill: Option<f64>) -> WorkerQueueSnapshot {
        WorkerQueueSnapshot {
            consumers: Vec::new(),
            streams: fill
                .into_iter()
                .map(|ratio| crate::bus::worker_consumers::StreamFill {
                    stream: TRAIN_TASTE_STREAM.name,
                    ratio,
                })
                .collect(),
        }
    }

    #[test]
    fn a_training_left_in_the_stream_blocks_another_export() {
        assert_eq!(holds_training(&snapshot_with(Some(0.001))), Some(true));
        assert_eq!(holds_training(&snapshot_with(Some(0.0))), Some(false));
        assert_eq!(holds_training(&snapshot_with(None)), None);
    }
}
