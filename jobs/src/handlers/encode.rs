use anyhow::{anyhow, ensure};
use backend_contracts::pipeline::{EncodeModel, EncodeResult};
use backend_contracts::reasons::{WorkerReason, WorkerStatus};
use backend_contracts::vector_store::query_vector_collection;

use crate::bus::DeliveryContext;
use crate::qdrant::QdrantProvisioner;
use crate::queue::{JobError, JobResult};

const QUERY_HASH_HEX_CHARS: usize = 64;

pub struct EncodeResultHandler {
    qdrant: QdrantProvisioner,
}

enum EncodeOutcome {
    Encoded {
        collection: &'static str,
        vector: Vec<f32>,
    },
    Unencoded {
        status: WorkerStatus,
        reason: WorkerReason,
    },
}

impl EncodeResultHandler {
    pub fn new(qdrant: QdrantProvisioner) -> Self {
        Self { qdrant }
    }

    pub async fn finish(&self, result: EncodeResult, delivery: DeliveryContext) -> JobResult {
        let model = result.model;
        let hash = result.hash.clone();
        match validate_result(result).map_err(JobError::permanent)? {
            EncodeOutcome::Encoded { collection, vector } => self
                .qdrant
                .upsert_query_vector(collection, &hash, vector)
                .await
                .map_err(JobError::retryable),
            EncodeOutcome::Unencoded { status, reason } => {
                tracing::debug!(
                    model = model.as_str(),
                    hash = %hash,
                    status = status.as_str(),
                    reason = reason.as_str(),
                    sequence = delivery.stream_sequence,
                    "encode result carried no vector and nothing was written"
                );
                Ok(())
            }
        }
    }
}

fn validate_result(result: EncodeResult) -> anyhow::Result<EncodeOutcome> {
    ensure!(
        is_query_hash(&result.hash),
        "encode result has an invalid query hash"
    );
    if result.status != WorkerStatus::Ok {
        let reason = result.reason.ok_or_else(|| {
            anyhow!(
                "encode result with status {} has no reason",
                result.status.as_str()
            )
        })?;
        ensure!(
            reason.status() == result.status,
            "encode reason {} does not belong to status {}",
            reason.as_str(),
            result.status.as_str()
        );
        return Ok(EncodeOutcome::Unencoded {
            status: result.status,
            reason,
        });
    }

    ensure!(
        result.reason.is_none(),
        "an ok encode result carries a reason"
    );
    let vector = result
        .vector
        .ok_or_else(|| anyhow!("an ok encode result has no vector"))?;
    validate_vector(result.model, &vector)?;
    let collection = query_vector_collection(result.model.as_str())
        .ok_or_else(|| anyhow!("encode model {} has no collection", result.model.as_str()))?;
    Ok(EncodeOutcome::Encoded { collection, vector })
}

fn validate_vector(model: EncodeModel, vector: &[f32]) -> anyhow::Result<()> {
    let dimensions = usize::try_from(model.dimensions())
        .map_err(|_| anyhow!("{} dimensions do not fit this platform", model.as_str()))?;
    ensure!(
        vector.len() == dimensions,
        "{} vector has {} values, expected {dimensions}",
        model.as_str(),
        vector.len()
    );
    ensure!(
        vector.iter().all(|value| value.is_finite()),
        "encode result vector contains a non-finite value"
    );
    Ok(())
}

fn is_query_hash(hash: &str) -> bool {
    hash.len() == QUERY_HASH_HEX_CHARS
        && hash
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use backend_contracts::pipeline::Producer;

    use super::*;

    const HASH: &str = "9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08";

    fn producer() -> Producer {
        Producer {
            worker_id: "gpu-main".to_owned(),
            build: "test".to_owned(),
            models: BTreeMap::new(),
            sync_version: None,
        }
    }

    fn encoded(model: EncodeModel, dimensions: usize) -> EncodeResult {
        EncodeResult {
            model,
            hash: HASH.to_owned(),
            status: WorkerStatus::Ok,
            reason: None,
            detail: None,
            producer: producer(),
            vector: Some(vec![0.5; dimensions]),
        }
    }

    fn unencoded(status: WorkerStatus, reason: Option<WorkerReason>) -> EncodeResult {
        EncodeResult {
            status,
            reason,
            vector: None,
            ..encoded(EncodeModel::Lyrics, 0)
        }
    }

    #[test]
    fn an_ok_result_writes_into_the_collection_of_its_model() -> anyhow::Result<()> {
        for model in [EncodeModel::Mulan, EncodeModel::Lyrics] {
            let dimensions = usize::try_from(model.dimensions())?;
            let EncodeOutcome::Encoded { collection, vector } =
                validate_result(encoded(model, dimensions))?
            else {
                anyhow::bail!("an ok result must be written");
            };
            assert_eq!(Some(collection), query_vector_collection(model.as_str()));
            assert_eq!(vector.len(), dimensions);
        }
        Ok(())
    }

    #[test]
    fn an_empty_text_writes_nothing() -> anyhow::Result<()> {
        let outcome = validate_result(unencoded(
            WorkerStatus::Empty,
            Some(WorkerReason::EmptyText),
        ))?;

        assert!(matches!(
            outcome,
            EncodeOutcome::Unencoded {
                status: WorkerStatus::Empty,
                reason: WorkerReason::EmptyText
            }
        ));
        Ok(())
    }

    #[test]
    fn a_failed_encode_writes_nothing() -> anyhow::Result<()> {
        let outcome = validate_result(unencoded(
            WorkerStatus::Failed,
            Some(WorkerReason::HashMismatch),
        ))?;

        assert!(matches!(outcome, EncodeOutcome::Unencoded { .. }));
        Ok(())
    }

    #[test]
    fn a_result_that_breaks_the_contract_is_refused() {
        let wrong_dimensions = encoded(EncodeModel::Mulan, 1024);
        let mut non_finite = encoded(EncodeModel::Lyrics, 1024);
        if let Some(first) = non_finite
            .vector
            .as_mut()
            .and_then(|vector| vector.first_mut())
        {
            *first = f32::NAN;
        }
        let mut missing_vector = encoded(EncodeModel::Lyrics, 1024);
        missing_vector.vector = None;
        let mut reason_on_ok = encoded(EncodeModel::Lyrics, 1024);
        reason_on_ok.reason = Some(WorkerReason::EmptyText);
        let mut bad_hash = encoded(EncodeModel::Lyrics, 1024);
        bad_hash.hash = HASH.to_uppercase();

        for invalid in [
            wrong_dimensions,
            non_finite,
            missing_vector,
            reason_on_ok,
            bad_hash,
            unencoded(WorkerStatus::Empty, None),
            unencoded(WorkerStatus::Empty, Some(WorkerReason::HashMismatch)),
        ] {
            assert!(validate_result(invalid).is_err());
        }
    }
}
