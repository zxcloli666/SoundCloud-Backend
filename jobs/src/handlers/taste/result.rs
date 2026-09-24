use std::collections::HashSet;
use std::time::Duration;

use anyhow::{Context, bail, ensure};
use backend_contracts::pipeline::{TASTE_DATA_BUCKET, TASTE_MODELS_BUCKET, TasteTrainResult};
use backend_contracts::reasons::{WorkerReason, WorkerStatus};
use backend_contracts::vector_store::TRACKS_TASTE_DIMENSIONS;
use backend_contracts::worker_contract::WorkerLane;
use chrono::{DateTime, Utc};
use tracing::{info, warn};

use crate::bus::StoredObject;
use crate::qdrant::TRACKS_TASTE;
use crate::queue::{JobError, JobResult};

use super::artifact::{self, TOWER_SUFFIX, TasteArtifact, is_version_name, tower_object};
use super::metrics::{ResultOutcome, record_orphan_removed, record_result, record_user_vectors};
use super::{EXPORT_RETRY, TasteHandler, object_error, vectors};

const ORPHAN_MARGIN_S: u64 = 86_400;

pub(crate) type TasteResult = TasteTrainResult;

#[derive(Debug, PartialEq, Eq)]
enum Verdict {
    Trained { version: String, items: u64 },
    Reopen,
    Untrained,
}

impl TasteHandler {
    pub async fn finish(&self, result: TasteResult) -> JobResult {
        match self.settle(&result).await {
            Ok(()) => {
                self.remove_object(TASTE_DATA_BUCKET, &result.input_object)
                    .await;
                Ok(())
            }
            Err(error) if error.is_retryable() => Err(error),
            Err(error) => {
                self.discard(&result, &error).await;
                Err(error)
            }
        }
    }

    async fn settle(&self, result: &TasteResult) -> JobResult {
        match judge(result).map_err(JobError::permanent)? {
            Verdict::Trained { version, items } => self.apply(result, &version, items).await,
            Verdict::Reopen => self.reopen(result).await,
            Verdict::Untrained => {
                note_untrained(result);
                Ok(())
            }
        }
    }

    async fn discard(&self, result: &TasteResult, error: &JobError) {
        record_result(ResultOutcome::Invalid);
        warn!(
            input = result.input_object,
            version = result.version.as_deref(),
            %error,
            "taste result cannot be applied; its objects go and a fresh export follows"
        );
        self.remove_object(TASTE_DATA_BUCKET, &result.input_object)
            .await;
        match self.known_versions().await {
            Ok(known) => {
                if let Some(version) = unrecorded_version(result, &known) {
                    self.remove_model(version).await;
                }
            }
            Err(error) => warn!(
                %error,
                "taste versions could not be read; the model objects wait for the orphan sweep"
            ),
        }
        if let Err(error) = self.schedule_export(EXPORT_RETRY).await {
            warn!(%error, "the next taste export could not be brought forward");
        }
    }

    async fn known_versions(&self) -> JobResult<HashSet<String>> {
        let versions = sqlx::query_file_scalar!("queries/taste/known_versions.sql")
            .fetch_all(&self.pool)
            .await
            .map_err(JobError::retryable)?;
        Ok(versions.into_iter().collect())
    }

    async fn remove_model(&self, version: &str) {
        self.remove_object(TASTE_MODELS_BUCKET, version).await;
        self.remove_object(TASTE_MODELS_BUCKET, &tower_object(version))
            .await;
    }

    async fn reopen(&self, result: &TasteResult) -> JobResult {
        self.schedule_export(Duration::ZERO).await?;
        record_result(ResultOutcome::Reopened);
        warn!(
            input = result.input_object,
            reason = result.reason.map(WorkerReason::as_str),
            detail = result.detail.as_deref(),
            "taste training was interrupted; a fresh export is due now"
        );
        Ok(())
    }

    async fn apply(&self, result: &TasteResult, version: &str, items: u64) -> JobResult {
        if let Some(recorded) = self.recorded(&result.input_object).await?
            && (recorded.version != version || recorded.applied_at.is_some())
        {
            record_result(ResultOutcome::Duplicate);
            if recorded.version != version {
                self.remove_model(version).await;
            }
            info!(
                input = result.input_object,
                version,
                recorded = recorded.version,
                "taste result for this input is already applied; nothing to do"
            );
            return Ok(());
        }

        let artifact = self.load_artifact(version, items).await?;
        let collection = collection_name(version);
        self.qdrant
            .ensure_versioned_collection(TRACKS_TASTE, &collection)
            .await
            .map_err(JobError::retryable)?;
        let items_written = self
            .qdrant
            .upsert_versioned_points(TRACKS_TASTE, &collection, artifact.item_points())
            .await
            .map_err(JobError::retryable)?;

        let computed_at = Utc::now();
        let users = vectors::pool_everyone(
            &self.pool,
            self.config.history_days,
            &artifact.pooling,
            &artifact.items,
            computed_at.timestamp(),
        )
        .await?;
        self.record_version(result, version, &collection, &artifact, users.len())
            .await?;
        sqlx::query_file!("queries/taste/clear_vectors.sql", version)
            .execute(&self.pool)
            .await
            .map_err(JobError::retryable)?;
        vectors::store(&self.pool, version, &users).await?;
        record_user_vectors(users.len());

        if activate(&self.pool, version, artifact.trained_at, computed_at).await? {
            self.follow_active_alias().await;
            record_result(ResultOutcome::Applied);
            info!(
                version,
                collection,
                items = items_written,
                users = users.len(),
                recall_at_50 = result.metrics.as_ref().map(|metrics| metrics.recall_at_50),
                ndcg_at_20 = result.metrics.as_ref().map(|metrics| metrics.ndcg_at_20),
                "taste version serves recommendations"
            );
        } else {
            record_result(ResultOutcome::Superseded);
            info!(
                version,
                "taste version is stored but a newer one keeps serving"
            );
        }
        self.prune().await;
        Ok(())
    }

    async fn follow_active_alias(&self) {
        let moved = match self.active_version().await {
            Ok(Some(active)) => self.serve_alias_of(&active).await,
            Ok(None) => Ok(()),
            Err(error) => Err(error),
        };
        if let Err(error) = moved {
            warn!(%error, "taste alias is left for the refresh tick to move");
        }
    }

    async fn recorded(&self, input_object: &str) -> JobResult<Option<RecordedInput>> {
        let row = sqlx::query_file!("queries/taste/recorded_input.sql", input_object)
            .fetch_optional(&self.pool)
            .await
            .map_err(JobError::retryable)?;
        Ok(row.map(|row| RecordedInput {
            version: row.version,
            applied_at: row.applied_at,
        }))
    }

    async fn load_artifact(&self, version: &str, items: u64) -> JobResult<TasteArtifact> {
        let limit = self.max_object_bytes().await?;
        let bytes = self
            .bus
            .read_object(TASTE_MODELS_BUCKET, version, limit)
            .await
            .map_err(object_error)?;
        let version = version.to_owned();
        tokio::task::spawn_blocking(move || artifact::parse(&bytes, &version, items))
            .await
            .map_err(JobError::retryable)?
            .map_err(JobError::permanent)
    }

    async fn record_version(
        &self,
        result: &TasteResult,
        version: &str,
        collection: &str,
        artifact: &TasteArtifact,
        users: usize,
    ) -> JobResult {
        let metrics = serde_json::json!({
            "offline": result.metrics,
            "details": artifact.metrics,
        });
        let dim = i16::try_from(TRACKS_TASTE_DIMENSIONS).map_err(JobError::permanent)?;
        let items = i32::try_from(artifact.items.len()).map_err(JobError::permanent)?;
        let users = i32::try_from(users).map_err(JobError::permanent)?;
        sqlx::query_file!(
            "queries/taste/record_version.sql",
            version,
            result.input_object,
            collection,
            artifact.trained_at,
            dim,
            artifact.pooling_json,
            metrics,
            items,
            users
        )
        .execute(&self.pool)
        .await
        .map_err(JobError::retryable)?;
        Ok(())
    }

    async fn prune(&self) {
        if let Err(error) = self.forget_stale_versions().await {
            warn!(%error, "stale taste versions could not be listed");
        }
        if let Err(error) = self.forget_orphan_objects().await {
            warn!(%error, "orphaned taste objects could not be listed");
        }
    }

    async fn forget_stale_versions(&self) -> JobResult {
        let kept_beside_active = i64::try_from(self.config.keep_versions.saturating_sub(1))
            .map_err(JobError::permanent)?;
        let stale: Vec<StaleVersion> =
            sqlx::query_file!("queries/taste/stale_versions.sql", kept_beside_active)
                .fetch_all(&self.pool)
                .await
                .map_err(JobError::retryable)?
                .into_iter()
                .map(|row| StaleVersion {
                    version: row.version,
                    collection: row.collection,
                })
                .collect();
        let served = self
            .qdrant
            .alias_target(TRACKS_TASTE)
            .await
            .map_err(JobError::retryable)?;
        let (droppable, behind_alias) = split_by_alias(stale, served.as_deref());
        for version in behind_alias {
            warn!(
                version = version.version,
                "stale taste version still serves through the alias; it stays"
            );
        }
        for version in droppable {
            if let Err(error) = self.forget_version(&version).await {
                warn!(%error, version = version.version, "stale taste version stays until the next prune");
            }
        }
        Ok(())
    }

    async fn forget_version(&self, version: &StaleVersion) -> JobResult {
        self.qdrant
            .drop_versioned_collection(TRACKS_TASTE, &version.collection)
            .await
            .map_err(JobError::retryable)?;
        sqlx::query_file!("queries/taste/forget_version.sql", version.version)
            .execute(&self.pool)
            .await
            .map_err(JobError::retryable)?;
        self.remove_model(&version.version).await;
        info!(version = version.version, "stale taste version removed");
        Ok(())
    }

    async fn forget_orphan_objects(&self) -> JobResult {
        let objects = self
            .bus
            .list_objects(TASTE_MODELS_BUCKET)
            .await
            .map_err(object_error)?;
        let known = self.known_versions().await?;
        for name in orphan_objects(&objects, &known, Utc::now().timestamp(), orphan_age_s()) {
            self.remove_object(TASTE_MODELS_BUCKET, &name).await;
            record_orphan_removed();
            info!(object = name, "orphaned taste object removed");
        }
        Ok(())
    }
}

struct RecordedInput {
    version: String,
    applied_at: Option<DateTime<Utc>>,
}

#[derive(Debug, PartialEq, Eq)]
struct StaleVersion {
    version: String,
    collection: String,
}

fn split_by_alias(
    stale: Vec<StaleVersion>,
    served: Option<&str>,
) -> (Vec<StaleVersion>, Vec<StaleVersion>) {
    stale
        .into_iter()
        .partition(|version| served != Some(version.collection.as_str()))
}

fn orphan_objects(
    objects: &[StoredObject],
    known: &HashSet<String>,
    now_unix: i64,
    min_age_s: i64,
) -> Vec<String> {
    objects
        .iter()
        .filter(|object| {
            let version = object
                .name
                .strip_suffix(TOWER_SUFFIX)
                .unwrap_or(&object.name);
            is_version_name(version)
                && !known.contains(version)
                && object
                    .modified_unix
                    .is_some_and(|modified| now_unix.saturating_sub(modified) >= min_age_s)
        })
        .map(|object| object.name.clone())
        .collect()
}

fn orphan_age_s() -> i64 {
    let window = WorkerLane::Taste
        .spec()
        .quarantine_after_s()
        .unwrap_or(ORPHAN_MARGIN_S);
    i64::try_from(window.saturating_add(ORPHAN_MARGIN_S)).unwrap_or(i64::MAX)
}

fn unrecorded_version<'a>(result: &'a TasteResult, known: &HashSet<String>) -> Option<&'a str> {
    result
        .version
        .as_deref()
        .filter(|version| is_version_name(version) && !known.contains(*version))
}

async fn activate(
    pool: &sqlx::PgPool,
    version: &str,
    trained_at: DateTime<Utc>,
    computed_at: DateTime<Utc>,
) -> JobResult<bool> {
    let mut transaction = pool.begin().await.map_err(JobError::retryable)?;
    sqlx::query(include_str!("../../../queries/taste/lock_activation.sql"))
        .execute(&mut *transaction)
        .await
        .map_err(JobError::retryable)?;
    let newer = sqlx::query_file_scalar!("queries/taste/newer_active.sql", version, trained_at)
        .fetch_one(&mut *transaction)
        .await
        .map_err(JobError::retryable)?;
    if newer {
        sqlx::query_file!("queries/taste/mark_applied.sql", version)
            .execute(&mut *transaction)
            .await
            .map_err(JobError::retryable)?;
    } else {
        sqlx::query_file!("queries/taste/retire_active.sql", version)
            .execute(&mut *transaction)
            .await
            .map_err(JobError::retryable)?;
        sqlx::query_file!(
            "queries/taste/activate_version.sql",
            version,
            computed_at.naive_utc()
        )
        .execute(&mut *transaction)
        .await
        .map_err(JobError::retryable)?;
    }
    transaction.commit().await.map_err(JobError::retryable)?;
    Ok(!newer)
}

pub(super) fn collection_name(version: &str) -> String {
    let stamp = version.strip_prefix("taste-").unwrap_or(version);
    format!("{}{}", TRACKS_TASTE.prefix, stamp.replace('-', "_"))
}

fn judge(result: &TasteResult) -> anyhow::Result<Verdict> {
    ensure!(
        result.dim == TRACKS_TASTE_DIMENSIONS,
        "taste result has {} dimensions, the contract requires {TRACKS_TASTE_DIMENSIONS}",
        result.dim
    );
    check_reason(result.status, result.reason)?;
    if result.status != WorkerStatus::Ok {
        let reopen = result
            .reason
            .is_some_and(|reason| reason.is_reopenable_on(WorkerLane::Taste));
        return Ok(if reopen {
            Verdict::Reopen
        } else {
            Verdict::Untrained
        });
    }
    let version = result
        .version
        .as_deref()
        .context("taste result with status ok names no version")?;
    ensure!(
        is_version_name(version),
        "taste result names version {version}, which breaks the contract pattern"
    );
    ensure!(
        result.object.as_deref() == Some(version),
        "taste result names model object {:?}, the contract requires {version}",
        result.object
    );
    let items = result
        .items_count
        .context("taste result with status ok has no items_count")?;
    ensure!(items > 0, "taste result with status ok has no items");
    ensure!(
        result.users_count.is_some() && result.metrics.is_some(),
        "taste result with status ok lacks users_count or metrics"
    );
    Ok(Verdict::Trained {
        version: version.to_owned(),
        items,
    })
}

fn check_reason(status: WorkerStatus, reason: Option<WorkerReason>) -> anyhow::Result<()> {
    let reason = match (status, reason) {
        (WorkerStatus::Ok, None) => return Ok(()),
        (WorkerStatus::Ok, Some(reason)) => bail!(
            "taste result with status ok carries reason {}",
            reason.as_str()
        ),
        (status, None) => bail!("taste result with status {} has no reason", status.as_str()),
        (_, Some(reason)) => reason,
    };
    ensure!(
        reason.status() == status,
        "taste reason {} does not belong to status {}",
        reason.as_str(),
        status.as_str()
    );
    ensure!(
        reason.is_published_on(WorkerLane::Taste),
        "taste result carries reason {} that its lane never publishes",
        reason.as_str()
    );
    Ok(())
}

fn note_untrained(result: &TasteResult) {
    let input = result.input_object.as_str();
    let reason = result.reason.map(WorkerReason::as_str);
    let detail = result.detail.as_deref();
    match result.status {
        WorkerStatus::Rejected => {
            record_result(ResultOutcome::Rejected);
            info!(
                input,
                reason,
                detail,
                "taste model did not beat its baselines; the serving version is kept"
            );
        }
        WorkerStatus::Empty => {
            record_result(ResultOutcome::Empty);
            info!(
                input,
                reason, detail, "taste training found too little to learn from"
            );
        }
        WorkerStatus::Missing => {
            record_result(ResultOutcome::Missing);
            warn!(
                input,
                reason, detail, "taste training could not read its input"
            );
        }
        WorkerStatus::Failed | WorkerStatus::Ok => {
            record_result(ResultOutcome::Failed);
            warn!(
                input,
                reason, detail, "taste training failed; the serving version is kept"
            );
        }
    }
}

#[cfg(test)]
#[path = "result_tests.rs"]
mod tests;
