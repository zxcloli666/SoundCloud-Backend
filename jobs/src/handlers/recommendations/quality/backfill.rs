use std::time::{Duration, Instant};

use sqlx::PgPool;
use tracing::info;

use crate::queue::{JobError, JobResult};

use super::QualityHandler;
use super::features::{QualityFeatures, load_features};
use super::store::{self, StoredModel};

pub(super) const BATCH_SIZE: i64 = 500;
const RUN_BUDGET: Duration = Duration::from_secs(4 * 60);

impl QualityHandler {
    pub async fn backfill(&self) -> JobResult {
        let current = store::latest(&self.pool).await?;
        let started = Instant::now();
        let mut scored = 0usize;
        while started.elapsed() < RUN_BUDGET {
            let batch = self.backfill_batch(current.as_ref()).await?;
            scored += batch.scored;
            if batch.scored == 0 || batch.picked < BATCH_SIZE {
                break;
            }
        }
        if scored > 0 {
            info!(
                tracks = scored,
                model_version = current.as_ref().map(|stored| stored.version),
                "recommendation quality scores backfilled"
            );
        }
        Ok(())
    }

    async fn backfill_batch(&self, current: Option<&StoredModel>) -> JobResult<Batch> {
        let version = current.map(|stored| stored.version);
        let ids = tracks_to_score(&self.pool, version).await?;
        let picked = i64::try_from(ids.len()).unwrap_or(BATCH_SIZE);
        if ids.is_empty() {
            return Ok(Batch { picked, scored: 0 });
        }
        let (ids, scores): (Vec<String>, Vec<f32>) = load_features(&self.pool, &self.qdrant, ids)
            .await?
            .into_iter()
            .map(|(id, features)| (id, score(current, &features)))
            .unzip();
        if !ids.is_empty() {
            persist(&self.pool, &ids, &scores, version).await?;
        }
        Ok(Batch {
            picked,
            scored: ids.len(),
        })
    }
}

struct Batch {
    picked: i64,
    scored: usize,
}

pub(super) async fn tracks_to_score(pool: &PgPool, version: Option<i64>) -> JobResult<Vec<String>> {
    let mut ids = sqlx::query_file_scalar!(
        "queries/recommendations/quality/select_missing_scores.sql",
        BATCH_SIZE
    )
    .fetch_all(pool)
    .await
    .map_err(JobError::retryable)?;
    let Some(version) = version else {
        return Ok(ids);
    };
    let room = BATCH_SIZE.saturating_sub(i64::try_from(ids.len()).unwrap_or(BATCH_SIZE));
    if room > 0 {
        let stale = sqlx::query_file_scalar!(
            "queries/recommendations/quality/select_stale_scores.sql",
            version,
            room
        )
        .fetch_all(pool)
        .await
        .map_err(JobError::retryable)?;
        ids.extend(stale);
    }
    Ok(ids)
}

pub(super) async fn persist(
    pool: &PgPool,
    ids: &[String],
    scores: &[f32],
    version: Option<i64>,
) -> JobResult {
    sqlx::query_file!(
        "queries/recommendations/quality/persist_scores.sql",
        ids,
        scores,
        version
    )
    .execute(pool)
    .await
    .map_err(JobError::retryable)?;
    Ok(())
}

fn score(current: Option<&StoredModel>, features: &QualityFeatures) -> f32 {
    match current {
        Some(stored) => stored.model.score(features),
        None => features.fallback_score(),
    }
}
