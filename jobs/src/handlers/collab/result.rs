use std::collections::HashSet;

use anyhow::{bail, ensure};
use backend_contracts::pipeline::{COLLAB_DATA_BUCKET, CollabTrainResult, collab_vectors_object};
use backend_contracts::reasons::{WorkerReason, WorkerStatus};
use backend_contracts::vector_store::TRACKS_COLLAB_DIMENSIONS;
use backend_contracts::worker_contract::WorkerLane;
use backend_contracts::{CollabTrainPayload, JobKind, Versioned};
use chrono::Utc;
use serde::Deserialize;
use tracing::{info, warn};
use uuid::Uuid;

use crate::bus::ObjectStoreError;
use crate::metrics::{CollabSkipReason, record_collab_skipped};
use crate::queue::{JobError, JobResult, NewJob, QueueError};

use super::{CollabHandler, object_error};

pub(crate) type CollabResult = CollabTrainResult;

const REOPEN_NAMESPACE: Uuid = Uuid::from_u128(0x5c0e_71a4_9d2b_4f6e_8a13_c7b0_e2d4_1f96);
const REOPEN_DEDUP_KEY: &str = "reopen";
const REOPEN_PRIORITY: i16 = -10;
const REOPEN_MAX_ATTEMPTS: i16 = 4;

#[derive(Deserialize)]
struct CollabBlob {
    dim: u64,
    points: Vec<CollabPoint>,
    #[serde(default)]
    metrics: Option<CollabMetrics>,
}

#[derive(Deserialize)]
struct CollabPoint {
    id: u64,
    #[serde(rename = "vec")]
    vector: Vec<f32>,
}

#[derive(Deserialize)]
struct CollabMetrics {
    hr_at_20: f64,
    popularity_hr_at_20: f64,
    sessions: u64,
    vocab: u64,
}

type CollabVectors = Vec<(u64, Vec<f32>)>;

impl CollabHandler {
    pub async fn finish(&self, result: CollabResult) -> JobResult {
        match vectors_object_of(&result).map_err(JobError::permanent)? {
            Some(vectors_object) => {
                self.apply(&result, &vectors_object).await?;
                self.remove_object(&vectors_object).await;
            }
            None if needs_reopen(&result) => self.reopen(&result).await?,
            None => note_untrained(&result),
        }
        self.remove_object(&result.input_object).await;
        Ok(())
    }

    async fn reopen(&self, result: &CollabResult) -> JobResult {
        let job = reopen_job(&result.input_object)?;
        self.queue
            .enqueue_if_absent(&job)
            .await
            .map_err(queue_error)?;
        warn!(
            input = result.input_object,
            reason = result.reason.map(WorkerReason::as_str),
            detail = result.detail.as_deref(),
            job = %job.id,
            "collab training was interrupted; another training is queued"
        );
        Ok(())
    }

    async fn apply(&self, result: &CollabResult, vectors_object: &str) -> JobResult {
        let payload = match self
            .bus
            .read_object(
                COLLAB_DATA_BUCKET,
                vectors_object,
                self.config.max_object_bytes,
            )
            .await
        {
            Ok(payload) => payload,
            Err(ObjectStoreError::NotFound { .. }) => {
                info!(
                    input = result.input_object,
                    vectors = vectors_object,
                    "collab vectors are already applied or expired; nothing to do"
                );
                return Ok(());
            }
            Err(error) => return Err(object_error(error)),
        };
        let mut blob =
            tokio::task::spawn_blocking(move || serde_json::from_slice::<CollabBlob>(&payload))
                .await
                .map_err(JobError::retryable)?
                .map_err(JobError::permanent)?;
        let metrics = blob.metrics.take();
        let points = validate_blob(blob, result.points_count).map_err(JobError::permanent)?;
        let model = result.input_object.as_str();
        let count = self
            .qdrant
            .upsert_collab(TRACKS_COLLAB_DIMENSIONS, model, points)
            .await
            .map_err(JobError::retryable)?;
        self.qdrant
            .remove_other_collab_models(model)
            .await
            .map_err(JobError::retryable)?;
        info!(
            count,
            dimensions = TRACKS_COLLAB_DIMENSIONS,
            input = result.input_object,
            hr_at_20 = metrics.as_ref().map(|metrics| metrics.hr_at_20),
            popularity_hr_at_20 = metrics.as_ref().map(|metrics| metrics.popularity_hr_at_20),
            sessions = metrics.as_ref().map(|metrics| metrics.sessions),
            vocab = metrics.as_ref().map(|metrics| metrics.vocab),
            "collab vectors stored"
        );
        Ok(())
    }

    async fn remove_object(&self, name: &str) {
        match self.bus.delete_object(COLLAB_DATA_BUCKET, name).await {
            Ok(()) | Err(ObjectStoreError::NotFound { .. }) => {}
            Err(error) => warn!(object = name, %error, "collab object could not be removed"),
        }
    }
}

fn vectors_object_of(result: &CollabResult) -> anyhow::Result<Option<String>> {
    let succeeded = result.status == WorkerStatus::Ok;
    ensure!(
        result.trained == succeeded,
        "collab result for {} says trained={} with status {}",
        result.input_object,
        result.trained,
        result.status.as_str()
    );
    ensure!(
        result.dim == TRACKS_COLLAB_DIMENSIONS,
        "collab result has {} dimensions, the contract requires {TRACKS_COLLAB_DIMENSIONS}",
        result.dim
    );
    check_reason(result.status, result.reason)?;
    if !succeeded {
        return Ok(None);
    }
    let expected = collab_vectors_object(&result.input_object);
    ensure!(
        result.object.as_deref() == Some(expected.as_str()),
        "collab result names vectors {:?}, the contract requires {expected}",
        result.object
    );
    Ok(Some(expected))
}

fn check_reason(status: WorkerStatus, reason: Option<WorkerReason>) -> anyhow::Result<()> {
    let reason = match (status, reason) {
        (WorkerStatus::Ok, None) => return Ok(()),
        (WorkerStatus::Ok, Some(reason)) => {
            bail!(
                "collab result with status ok carries reason {}",
                reason.as_str()
            )
        }
        (status, None) => bail!(
            "collab result with status {} has no reason",
            status.as_str()
        ),
        (_, Some(reason)) => reason,
    };
    ensure!(
        reason.status() == status,
        "collab reason {} does not belong to status {}",
        reason.as_str(),
        status.as_str()
    );
    ensure!(
        reason.is_published_on(WorkerLane::Collab),
        "collab result carries reason {} that its lane never publishes",
        reason.as_str()
    );
    Ok(())
}

fn needs_reopen(result: &CollabResult) -> bool {
    result
        .reason
        .is_some_and(|reason| reason.is_reopenable_on(WorkerLane::Collab))
}

fn reopen_job(input_object: &str) -> JobResult<NewJob> {
    let payload = Versioned::V1(CollabTrainPayload::default());
    Ok(NewJob {
        id: Uuid::new_v5(&REOPEN_NAMESPACE, input_object.as_bytes()),
        kind: JobKind::CollabTrain,
        dedup_key: Some(REOPEN_DEDUP_KEY.to_owned()),
        payload: serde_json::to_value(payload).map_err(JobError::permanent)?,
        priority: REOPEN_PRIORITY,
        max_attempts: REOPEN_MAX_ATTEMPTS,
        available_at: Utc::now(),
    })
}

fn queue_error(error: QueueError) -> JobError {
    match error {
        QueueError::Database(error) => JobError::retryable(error),
        error => JobError::permanent(error),
    }
}

fn note_untrained(result: &CollabResult) {
    let reason = result.reason.map(WorkerReason::as_str);
    let input = result.input_object.as_str();
    let detail = result.detail.as_deref();
    match result.reason {
        Some(WorkerReason::BelowBaseline) => {
            record_collab_skipped(CollabSkipReason::BelowBaseline);
            info!(
                input,
                detail, "collab model is worse than popularity; stored vectors are kept"
            );
        }
        Some(WorkerReason::EmptyVocab) => {
            record_collab_skipped(CollabSkipReason::EmptyVocab);
            info!(input, detail, "collab training found no vocabulary");
        }
        _ => warn!(
            input,
            status = result.status.as_str(),
            reason,
            detail,
            "collab training produced no model"
        ),
    }
}

fn validate_blob(blob: CollabBlob, announced_points: u64) -> anyhow::Result<CollabVectors> {
    ensure!(
        blob.dim == TRACKS_COLLAB_DIMENSIONS,
        "collab vectors have {} dimensions, the contract requires {TRACKS_COLLAB_DIMENSIONS}",
        blob.dim
    );
    let dimensions = usize::try_from(blob.dim)
        .map_err(|_| anyhow::anyhow!("collab result dimensions do not fit this platform"))?;
    ensure!(!blob.points.is_empty(), "collab result has no points");
    ensure!(
        u64::try_from(blob.points.len()).ok() == Some(announced_points),
        "collab vectors hold {} points, the result announced {announced_points}",
        blob.points.len()
    );

    let mut ids = HashSet::with_capacity(blob.points.len());
    let mut points = Vec::with_capacity(blob.points.len());
    for point in blob.points {
        ensure!(point.id > 0, "collab result contains a zero track id");
        ensure!(
            ids.insert(point.id),
            "collab result contains duplicate track ids"
        );
        ensure!(
            point.vector.len() == dimensions,
            "collab vector {} has {} values, expected {dimensions}",
            point.id,
            point.vector.len()
        );
        ensure!(
            point.vector.iter().all(|value| value.is_finite()),
            "collab vector {} contains a non-finite value",
            point.id
        );
        points.push((point.id, point.vector));
    }
    Ok(points)
}

#[cfg(test)]
#[path = "result_tests.rs"]
mod tests;
