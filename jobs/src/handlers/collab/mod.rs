mod result;
mod sessions;

use backend_contracts::pipeline::{
    COLLAB_DATA_BUCKET, COLLAB_DATASET_VERSION, CollabTrainRequest, TRAIN_COLLAB,
    TRAIN_COLLAB_STREAM,
};
use backend_contracts::vector_store::TRACKS_COLLAB_DIMENSIONS;
use backend_contracts::{COLLAB_MAX_MIN_COUNT, CollabTrainPayload};
use sqlx::PgPool;
use tracing::info;
use uuid::Uuid;

use crate::bus::{Bus, ObjectStoreError, WorkerQueueSnapshot};
use crate::config::CollabConfig;
use crate::metrics::{CollabSkipReason, record_collab_skipped};
use crate::qdrant::QdrantProvisioner;
use crate::queue::{JobError, JobRepository, JobResult};

pub(crate) use result::CollabResult;

const WINDOW: u32 = 5;
const EPOCHS: u32 = 5;
const NEGATIVE_SAMPLES: u32 = 10;

pub struct CollabHandler {
    pool: PgPool,
    queue: JobRepository,
    bus: Bus,
    qdrant: QdrantProvisioner,
    config: CollabConfig,
}

impl CollabHandler {
    pub fn new(pool: PgPool, bus: Bus, qdrant: QdrantProvisioner, config: CollabConfig) -> Self {
        Self {
            queue: JobRepository::new(pool.clone(), "collab".to_owned()),
            pool,
            bus,
            qdrant,
            config,
        }
    }

    pub async fn bootstrap(&self, job_id: Uuid) -> JobResult {
        self.ensure_contract_dimensions().await?;
        let points = self
            .qdrant
            .collab_points_count()
            .await
            .map_err(JobError::retryable)?;
        if points > 0 {
            info!(points, "collab collection is ready");
            return Ok(());
        }
        if self.training_in_flight().await? {
            info!("collab collection is empty but a training is already queued or running");
            record_collab_skipped(CollabSkipReason::InFlight);
            return Ok(());
        }
        self.dispatch(job_id, CollabTrainPayload::default()).await
    }

    async fn training_in_flight(&self) -> JobResult<bool> {
        let snapshot = self.bus.worker_queue_snapshot().await;
        holds_training(&snapshot).ok_or_else(|| {
            JobError::retryable(anyhow::anyhow!(
                "NATS stream {} is not readable",
                TRAIN_COLLAB_STREAM.name
            ))
        })
    }

    pub async fn train(&self, job_id: Uuid, payload: CollabTrainPayload) -> JobResult {
        self.ensure_contract_dimensions().await?;
        self.dispatch(job_id, payload).await
    }

    async fn dispatch(&self, job_id: Uuid, payload: CollabTrainPayload) -> JobResult {
        let min_count = payload.min_count.unwrap_or(self.config.min_count);
        if min_count == 0 || min_count > COLLAB_MAX_MIN_COUNT {
            return Err(JobError::permanent(anyhow::anyhow!(
                "collab minimum count {min_count} is outside 1..={COLLAB_MAX_MIN_COUNT}"
            )));
        }

        let object = input_object(job_id);
        if self.input_already_uploaded(&object).await? {
            info!(
                object = object.as_str(),
                "collab input is already uploaded; its training request is sent again"
            );
        } else if !self.upload_input(&object).await? {
            return Ok(());
        }
        let request = CollabTrainRequest {
            object: object.clone(),
            dataset_version: COLLAB_DATASET_VERSION,
            dim: TRACKS_COLLAB_DIMENSIONS,
            min_count,
            window: WINDOW,
            epochs: EPOCHS,
            negative: NEGATIVE_SAMPLES,
        };
        self.bus
            .publish_dedup(TRAIN_COLLAB, &request, &train_message_id(&object))
            .await
            .map_err(JobError::retryable)?;
        info!(
            min_count,
            object = object.as_str(),
            "collab training dispatched"
        );
        Ok(())
    }

    async fn input_already_uploaded(&self, object: &str) -> JobResult<bool> {
        let probe = self.bus.read_object(COLLAB_DATA_BUCKET, object, 0).await;
        is_uploaded(probe)
    }

    async fn upload_input(&self, object: &str) -> JobResult<bool> {
        let dataset = sessions::build(&self.pool, self.config.max_object_bytes).await?;
        if dataset.session_count < self.config.min_sessions {
            info!(
                sessions = dataset.session_count,
                events = dataset.event_count,
                minimum = self.config.min_sessions,
                "collab training skipped because the dataset is too small"
            );
            record_collab_skipped(CollabSkipReason::TooFewSessions);
            return Ok(false);
        }
        let mut file = dataset.open().await.map_err(JobError::retryable)?;
        self.bus
            .put_object_reader(COLLAB_DATA_BUCKET, object, &mut file)
            .await
            .map_err(object_error)?;
        info!(
            sessions = dataset.session_count,
            events = dataset.event_count,
            truncated = dataset.truncated,
            object,
            "collab input uploaded"
        );
        Ok(true)
    }

    async fn ensure_contract_dimensions(&self) -> JobResult {
        match self
            .qdrant
            .collab_dimension()
            .await
            .map_err(JobError::retryable)?
        {
            Some(existing) if existing != TRACKS_COLLAB_DIMENSIONS => {
                Err(JobError::permanent(anyhow::anyhow!(
                    "collab collection has {existing} dimensions, the contract requires {TRACKS_COLLAB_DIMENSIONS}"
                )))
            }
            _ => Ok(()),
        }
    }
}

fn holds_training(snapshot: &WorkerQueueSnapshot) -> Option<bool> {
    snapshot
        .streams
        .iter()
        .find(|fill| fill.stream == TRAIN_COLLAB_STREAM.name)
        .map(|fill| fill.ratio > 0.0)
}

fn is_uploaded(probe: Result<Vec<u8>, ObjectStoreError>) -> JobResult<bool> {
    match probe {
        Ok(_) | Err(ObjectStoreError::TooLarge { .. }) => Ok(true),
        Err(ObjectStoreError::NotFound { .. }) => Ok(false),
        Err(error) => Err(object_error(error)),
    }
}

fn input_object(job_id: Uuid) -> String {
    format!("collab-input-{job_id}")
}

fn train_message_id(object: &str) -> String {
    format!("collab:{object}")
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
    use super::*;

    #[test]
    fn a_training_is_deduplicated_by_its_input_object() {
        let job_id = Uuid::nil();
        let object = input_object(job_id);

        assert_eq!(object, "collab-input-00000000-0000-0000-0000-000000000000");
        assert_eq!(train_message_id(&object), format!("collab:{object}"));
    }

    fn snapshot_with(fill: Option<f64>) -> WorkerQueueSnapshot {
        WorkerQueueSnapshot {
            consumers: Vec::new(),
            streams: fill
                .into_iter()
                .map(|ratio| crate::bus::worker_consumers::StreamFill {
                    stream: TRAIN_COLLAB_STREAM.name,
                    ratio,
                })
                .collect(),
        }
    }

    #[test]
    fn a_retried_training_never_overwrites_an_input_the_worker_may_be_reading() {
        let object = input_object(Uuid::nil());
        let present = is_uploaded(Err(ObjectStoreError::TooLarge {
            bucket: COLLAB_DATA_BUCKET.to_owned(),
            name: object.clone(),
            actual: 4096,
            limit: 0,
        }));
        let empty = is_uploaded(Ok(Vec::new()));
        let absent = is_uploaded(Err(ObjectStoreError::NotFound {
            bucket: COLLAB_DATA_BUCKET.to_owned(),
            name: object,
        }));
        let unreachable = is_uploaded(Err(ObjectStoreError::Unavailable(anyhow::anyhow!(
            "NATS is down"
        ))));

        assert!(matches!(present, Ok(true)));
        assert!(matches!(empty, Ok(true)));
        assert!(matches!(absent, Ok(false)));
        assert!(unreachable.is_err_and(|error| error.is_retryable()));
    }

    #[test]
    fn a_training_left_in_the_stream_blocks_another_bootstrap() {
        assert_eq!(holds_training(&snapshot_with(Some(0.001))), Some(true));
        assert_eq!(holds_training(&snapshot_with(Some(0.0))), Some(false));
        assert_eq!(holds_training(&snapshot_with(None)), None);
    }
}
