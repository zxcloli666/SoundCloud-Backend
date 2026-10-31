#[cfg(test)]
#[path = "result_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "result_live_tests.rs"]
mod live_tests;

#[cfg(test)]
#[path = "result_lock_tests.rs"]
mod lock_tests;

use anyhow::{Context, anyhow, ensure};
use backend_contracts::pipeline::{AudioIndexRequest, AudioIndexResult};
use backend_contracts::reasons::{WorkerReason, WorkerStatus, outcome_rank};
use backend_contracts::vector_store::{TRACKS_CLAP_DIMENSIONS, TRACKS_MERT_DIMENSIONS};
use backend_contracts::worker_contract::WorkerLane;
use sqlx::PgPool;
use uuid::Uuid;

use crate::bus::DeliveryContext;
use crate::qdrant::QdrantProvisioner;
use crate::queue::{JobError, JobResult};

const MAX_FINGERPRINT_BYTES: usize = 64 * 1024;
const FINGERPRINT_PREFIX_CHARS: usize = 64;
const RESULT_CLAIM_SECONDS: i64 = 120;

pub struct AudioIndexResultHandler {
    pool: PgPool,
    qdrant: QdrantProvisioner,
}

enum AudioResult {
    Indexed(AudioIndex),
    Settled {
        outcome: AudioOutcome,
        detail: Option<String>,
    },
}

struct AudioIndex {
    sc_track_id: String,
    point_id: u64,
    upload_generation: i64,
    attempt: i32,
    mert: Vec<f32>,
    clap: Vec<f32>,
    fingerprint: Option<String>,
}

struct Committed {
    already_settled: bool,
    committed: bool,
    invalidated: bool,
    demoted: bool,
}

struct FingerprintLocks {
    track_id: Uuid,
    track_canonical: Option<Uuid>,
    neighbour: Option<Neighbour>,
}

impl FingerprintLocks {
    fn neighbour_moved(&self) -> bool {
        self.neighbour
            .as_ref()
            .is_some_and(|neighbour| !neighbour.unchanged)
    }
}

#[derive(sqlx::FromRow)]
struct LockedTrack {
    id: Uuid,
    sc_track_id: String,
    canonical_track_id: Option<Uuid>,
    prefix: Option<String>,
}

struct Neighbour {
    id: Uuid,
    canonical: Option<Uuid>,
    unchanged: bool,
}

struct AudioOutcome {
    sc_track_id: String,
    upload_generation: i64,
    attempt: i32,
    status: WorkerStatus,
    reason: WorkerReason,
}

impl AudioOutcome {
    fn wire_status(&self) -> &'static str {
        if self.reason.is_reopenable_on(WorkerLane::Audio) {
            "reopenable"
        } else {
            "terminal"
        }
    }

    fn rank(&self) -> i16 {
        i16::from(outcome_rank(self.status, Some(self.reason)))
    }
}

enum ResultClaim {
    Settled,
    Busy,
    Deferred,
    Owned { lease_id: Uuid },
}

impl AudioIndexResultHandler {
    pub fn new(pool: PgPool, qdrant: QdrantProvisioner) -> Self {
        Self { pool, qdrant }
    }

    pub async fn finish(&self, result: AudioIndexResult, delivery: DeliveryContext) -> JobResult {
        match validate_result(result).map_err(JobError::permanent)? {
            AudioResult::Indexed(index) => self.index(index, delivery).await,
            AudioResult::Settled { outcome, detail } => {
                let applied = self.settle(&outcome).await?;
                tracing::info!(
                    track = %outcome.sc_track_id,
                    generation = outcome.upload_generation,
                    attempt = outcome.attempt,
                    status = outcome.status.as_str(),
                    reason = outcome.reason.as_str(),
                    detail = detail.as_deref().unwrap_or_default(),
                    sequence = delivery.stream_sequence,
                    applied,
                    "audio index worker settled the task without vectors"
                );
                Ok(())
            }
        }
    }

    pub async fn apply_worker_lost(&self, stream_seq: u64, payload: &[u8]) -> JobResult {
        let outcome = worker_lost_outcome(payload).map_err(JobError::permanent)?;
        let applied = self.settle(&outcome).await?;
        tracing::info!(
            track = %outcome.sc_track_id,
            generation = outcome.upload_generation,
            attempt = outcome.attempt,
            stream_sequence = stream_seq,
            applied,
            "audio index task was lost by its worker"
        );
        Ok(())
    }

    async fn settle(&self, outcome: &AudioOutcome) -> JobResult<bool> {
        let settled = sqlx::query_file_scalar!(
            "queries/indexing/result/settle.sql",
            &outcome.sc_track_id,
            outcome.upload_generation,
            outcome.attempt,
            outcome.wire_status(),
            outcome.rank(),
            outcome.status.as_str(),
            outcome.reason.as_str()
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(JobError::retryable)?;
        Ok(settled.is_some())
    }

    async fn index(&self, mut index: AudioIndex, delivery: DeliveryContext) -> JobResult {
        let stream_sequence = i64::try_from(delivery.stream_sequence)
            .map_err(|_| JobError::permanent(anyhow!("NATS sequence is out of range")))?;
        let lease_id = match self.claim(&index, stream_sequence, &delivery).await? {
            ResultClaim::Settled => return Ok(()),
            ResultClaim::Busy => {
                return Err(JobError::retryable(anyhow!(
                    "audio index result is already being applied"
                )));
            }
            ResultClaim::Deferred => {
                return Err(JobError::retryable(anyhow!(
                    "audio index result waits for its track to become servable"
                )));
            }
            ResultClaim::Owned { lease_id } => lease_id,
        };

        self.qdrant
            .upsert_audio(
                index.point_id,
                index.upload_generation,
                std::mem::take(&mut index.mert),
                std::mem::take(&mut index.clap),
                None,
            )
            .await
            .map_err(JobError::retryable)?;

        let outcome = self
            .commit(&index, stream_sequence, &delivery, lease_id)
            .await?;
        if outcome.already_settled || outcome.committed {
            return Ok(());
        }
        if outcome.invalidated {
            tracing::warn!(
                track = %index.sc_track_id,
                generation = index.upload_generation,
                attempt = index.attempt,
                demoted = outcome.demoted,
                "audio index result was written after its epoch closed and was invalidated"
            );
            return Ok(());
        }
        Err(JobError::retryable(anyhow!(
            "audio index result lost its lease before commit for track {}",
            index.sc_track_id
        )))
    }

    async fn commit(
        &self,
        index: &AudioIndex,
        stream_sequence: i64,
        delivery: &DeliveryContext,
        lease_id: Uuid,
    ) -> JobResult<Committed> {
        let sc_track_id = index.sc_track_id.as_str();
        let fingerprint = index.fingerprint.as_deref();
        let prefix = fingerprint.map(|fingerprint| {
            fingerprint
                .chars()
                .take(FINGERPRINT_PREFIX_CHARS)
                .collect::<String>()
        });
        let mut tries = 0;
        loop {
            tries += 1;
            let mut transaction = self.pool.begin().await.map_err(JobError::retryable)?;
            let locks = match prefix.as_deref() {
                Some(prefix) => {
                    lock_fingerprint_rows(&mut transaction, sc_track_id, prefix).await?
                }
                None => None,
            };
            if tries == 1 && locks.as_ref().is_some_and(FingerprintLocks::neighbour_moved) {
                transaction.rollback().await.map_err(JobError::retryable)?;
                continue;
            }
            let outcome = sqlx::query_file!(
                "queries/indexing/result/commit.sql",
                &delivery.consumer,
                &delivery.stream,
                stream_sequence,
                delivery.published_at,
                sc_track_id,
                index.upload_generation,
                lease_id,
                index.attempt
            )
            .fetch_one(&mut *transaction)
            .await
            .map_err(JobError::retryable)?;
            if outcome.committed && (!outcome.indexed || !outcome.accepted) {
                transaction.rollback().await.map_err(JobError::retryable)?;
                return Err(JobError::retryable(anyhow!(
                    "audio index commit did not settle track {sc_track_id}"
                )));
            }
            if outcome.committed
                && let (Some(fingerprint), Some(locks)) = (fingerprint, locks.as_ref())
            {
                if locks.neighbour_moved() {
                    tracing::warn!(
                        track = %sc_track_id,
                        "fingerprint neighbour moved twice before its lock, no canonical link"
                    );
                }
                apply_fingerprint(&mut transaction, fingerprint, locks).await?;
            }
            transaction.commit().await.map_err(JobError::retryable)?;
            return Ok(Committed {
                already_settled: outcome.already_settled,
                committed: outcome.committed,
                invalidated: outcome.invalidated,
                demoted: outcome.demoted,
            });
        }
    }

    async fn claim(
        &self,
        index: &AudioIndex,
        stream_sequence: i64,
        delivery: &DeliveryContext,
    ) -> JobResult<ResultClaim> {
        let lease_id = Uuid::now_v7();
        let outcome = sqlx::query_file!(
            "queries/indexing/result/claim.sql",
            &delivery.consumer,
            &delivery.stream,
            stream_sequence,
            delivery.published_at,
            &index.sc_track_id,
            index.upload_generation,
            lease_id,
            RESULT_CLAIM_SECONDS,
            index.attempt
        )
        .fetch_one(&self.pool)
        .await
        .map_err(JobError::retryable)?;
        if !outcome.known {
            return Err(JobError::retryable(anyhow!(
                "audio index result references unknown track {}",
                index.sc_track_id
            )));
        }
        if outcome.claimed {
            return Ok(ResultClaim::Owned { lease_id });
        }
        if outcome.busy {
            return Ok(ResultClaim::Busy);
        }
        if outcome.deferred {
            return Ok(ResultClaim::Deferred);
        }
        if outcome.settled {
            if outcome.demoted {
                tracing::warn!(
                    track = %index.sc_track_id,
                    generation = index.upload_generation,
                    attempt = index.attempt,
                    "audio index result from a closed epoch invalidated the indexed track"
                );
            }
            return Ok(ResultClaim::Settled);
        }
        Err(JobError::retryable(anyhow!(
            "audio index result was neither claimed nor settled"
        )))
    }
}

async fn lock_fingerprint_rows(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    sc_track_id: &str,
    prefix: &str,
) -> JobResult<Option<FingerprintLocks>> {
    sqlx::query(include_str!(
        "../../../queries/indexing/result/lock_fingerprint.sql"
    ))
    .bind(prefix)
    .execute(&mut **transaction)
    .await
    .map_err(JobError::retryable)?;
    let seen = sqlx::query_as::<_, (Uuid, Option<Uuid>)>(include_str!(
        "../../../queries/indexing/result/find_neighbour.sql"
    ))
    .bind(prefix)
    .bind(sc_track_id)
    .fetch_optional(&mut **transaction)
    .await
    .map_err(JobError::retryable)?;
    let locked = sqlx::query_as::<_, LockedTrack>(include_str!(
        "../../../queries/indexing/result/lock_pair.sql"
    ))
    .bind(sc_track_id)
    .bind(seen.map(|(id, _)| id))
    .fetch_all(&mut **transaction)
    .await
    .map_err(JobError::retryable)?;
    let Some(track) = locked.iter().find(|row| row.sc_track_id == sc_track_id) else {
        return Ok(None);
    };
    let neighbour = seen.map(|(id, canonical)| Neighbour {
        id,
        canonical,
        unchanged: locked.iter().any(|row| {
            row.id == id
                && row.canonical_track_id == canonical
                && row.prefix.as_deref() == Some(prefix)
        }),
    });
    Ok(Some(FingerprintLocks {
        track_id: track.id,
        track_canonical: track.canonical_track_id,
        neighbour,
    }))
}

async fn apply_fingerprint(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    fingerprint: &str,
    locks: &FingerprintLocks,
) -> JobResult {
    sqlx::query(include_str!(
        "../../../queries/indexing/result/set_fingerprint.sql"
    ))
    .bind(locks.track_id)
    .bind(fingerprint)
    .execute(&mut **transaction)
    .await
    .map_err(JobError::retryable)?;
    let Some(neighbour) = locks.neighbour.as_ref().filter(|neighbour| neighbour.unchanged) else {
        return Ok(());
    };
    let Some(canonical_id) = joined_canonical_id(locks.track_canonical, neighbour.canonical) else {
        return Ok(());
    };
    sqlx::query(include_str!(
        "../../../queries/indexing/result/link_canonical.sql"
    ))
    .bind(canonical_id)
    .bind(locks.track_id)
    .bind(neighbour.id)
    .execute(&mut **transaction)
    .await
    .map_err(JobError::retryable)?;
    Ok(())
}

fn joined_canonical_id(current: Option<Uuid>, neighbour: Option<Uuid>) -> Option<Uuid> {
    match (current, neighbour) {
        (Some(current), Some(neighbour)) if current != neighbour => None,
        (Some(current), _) => Some(current),
        (None, Some(neighbour)) => Some(neighbour),
        (None, None) => Some(Uuid::now_v7()),
    }
}

fn validate_result(result: AudioIndexResult) -> anyhow::Result<AudioResult> {
    let point_id = canonical_point_id(&result.sc_track_id)?;
    ensure!(
        result.upload_generation > 0,
        "audio index result has an uncorrelated upload generation"
    );
    let attempt = wire_attempt(result.attempt)?;
    if result.status != WorkerStatus::Ok {
        let reason = settled_reason(result.status, result.reason)?;
        return Ok(AudioResult::Settled {
            outcome: AudioOutcome {
                sc_track_id: result.sc_track_id,
                upload_generation: result.upload_generation,
                attempt,
                status: result.status,
                reason,
            },
            detail: result.detail,
        });
    }

    ensure!(
        result.reason.is_none(),
        "an ok audio index result carries a reason"
    );
    let mert = result
        .mert
        .ok_or_else(|| anyhow!("an ok audio index result has no MERT vector"))?;
    let clap = result
        .clap
        .ok_or_else(|| anyhow!("an ok audio index result has no CLAP vector"))?;
    validate_vector(&mert, TRACKS_MERT_DIMENSIONS, "MERT")?;
    validate_vector(&clap, TRACKS_CLAP_DIMENSIONS, "CLAP")?;
    let fingerprint = result
        .fingerprint
        .map(|fingerprint| fingerprint.trim().to_owned())
        .filter(|fingerprint| !fingerprint.is_empty());
    ensure!(
        fingerprint.as_ref().is_none_or(|fingerprint| {
            fingerprint.len() <= MAX_FINGERPRINT_BYTES && !fingerprint.chars().any(char::is_control)
        }),
        "audio index result has an invalid fingerprint"
    );

    Ok(AudioResult::Indexed(AudioIndex {
        sc_track_id: result.sc_track_id,
        point_id,
        upload_generation: result.upload_generation,
        attempt,
        mert,
        clap,
        fingerprint,
    }))
}

fn worker_lost_outcome(payload: &[u8]) -> anyhow::Result<AudioOutcome> {
    let request = serde_json::from_slice::<AudioIndexRequest>(payload)
        .context("lost audio index task is not an index request")?;
    canonical_point_id(&request.sc_track_id)?;
    ensure!(
        request.upload_generation > 0,
        "lost audio index task has an uncorrelated upload generation"
    );
    Ok(AudioOutcome {
        attempt: wire_attempt(request.attempt)?,
        sc_track_id: request.sc_track_id,
        upload_generation: request.upload_generation,
        status: WorkerStatus::Failed,
        reason: WorkerReason::WorkerLost,
    })
}

fn settled_reason(
    status: WorkerStatus,
    reason: Option<WorkerReason>,
) -> anyhow::Result<WorkerReason> {
    let reason = reason.ok_or_else(|| {
        anyhow!(
            "audio index result with status {} has no reason",
            status.as_str()
        )
    })?;
    ensure!(
        reason.status() == status,
        "audio index reason {} does not belong to status {}",
        reason.as_str(),
        status.as_str()
    );
    ensure!(
        reason.is_published_on(WorkerLane::Audio),
        "audio index result carries reason {} that its lane never reopens",
        reason.as_str()
    );
    Ok(reason)
}

fn canonical_point_id(sc_track_id: &str) -> anyhow::Result<u64> {
    let point_id = sc_track_id
        .parse::<u64>()
        .map_err(|_| anyhow!("audio index result has an invalid track id"))?;
    ensure!(
        point_id > 0 && point_id.to_string() == sc_track_id,
        "audio index result has a non-canonical track id"
    );
    Ok(point_id)
}

fn wire_attempt(attempt: i64) -> anyhow::Result<i32> {
    i32::try_from(attempt)
        .ok()
        .filter(|attempt| *attempt > 0)
        .ok_or_else(|| anyhow!("audio index result has an uncorrelated attempt"))
}

fn validate_vector(vector: &[f32], dimensions: u64, name: &str) -> anyhow::Result<()> {
    let dimensions = usize::try_from(dimensions)
        .map_err(|_| anyhow!("{name} dimensions do not fit this platform"))?;
    ensure!(
        vector.len() == dimensions,
        "{name} vector has {} values, expected {dimensions}",
        vector.len()
    );
    ensure!(
        vector.iter().all(|value| value.is_finite()),
        "{name} vector contains a non-finite value"
    );
    Ok(())
}
