use std::collections::HashMap;

use chrono::Utc;
use sqlx::PgPool;
use tracing::{info, warn};

use crate::queue::{JobError, JobResult};

use super::QualityHandler;
use super::features::load_features;
use super::model::{self, LabeledTrack, MIN_EXAMPLES, QualityModel, TrainedModel};
use super::store;

const POSITIVE_EVENTS: &[&str] = &["like", "playlist_add", "full_play"];
const POSITIVE_HISTORY_DAYS: i64 = 180;
const POSITIVE_LISTENERS: i64 = 3;
const POSITIVE_LIMIT: i64 = 800;
const NEGATIVE_LIMIT: i64 = 500;
const MEANINGFUL_CHANGE: f32 = 0.05;

impl QualityHandler {
    pub async fn train(&self) -> JobResult {
        let labels = self.labels().await?;
        if labels.len() < MIN_EXAMPLES {
            info!(
                examples = labels.len(),
                "recommendation quality dataset is too small"
            );
            return Ok(());
        }

        let ids = labels.keys().cloned().collect::<Vec<_>>();
        let examples = load_features(&self.pool, &self.qdrant, ids)
            .await?
            .into_iter()
            .map(|(id, features)| LabeledTrack {
                positive: labels.get(&id).copied().unwrap_or(false),
                features,
            })
            .collect::<Vec<_>>();
        let outcome = tokio::task::spawn_blocking(move || model::train(&examples))
            .await
            .map_err(JobError::retryable)?;
        let trained = match outcome {
            Ok(trained) => trained,
            Err(refusal) => {
                info!(
                    reason = refusal.as_str(),
                    "recommendation quality model was not trained"
                );
                return Ok(());
            }
        };
        if !trained.converged {
            warn!(
                iterations = trained.iterations,
                "recommendation quality training stopped before it converged"
            );
        }

        let Some(version) = save_if_changed(&self.pool, &trained).await? else {
            info!(
                examples = trained.examples,
                "recommendation quality model is unchanged; the current version stays"
            );
            return Ok(());
        };
        info!(
            version,
            examples = trained.examples,
            positives = trained.positives,
            accuracy = trained.accuracy,
            iterations = trained.iterations,
            "recommendation quality model trained"
        );
        Ok(())
    }

    pub(super) async fn labels(&self) -> JobResult<HashMap<String, bool>> {
        let since = Utc::now().naive_utc() - chrono::Duration::days(POSITIVE_HISTORY_DAYS);
        let events = POSITIVE_EVENTS
            .iter()
            .map(|event| (*event).to_owned())
            .collect::<Vec<_>>();
        let positive_ids = sqlx::query_file_scalar!(
            "queries/recommendations/quality/select_positive_examples.sql",
            since,
            &events,
            POSITIVE_LISTENERS,
            POSITIVE_LIMIT
        )
        .fetch_all(&self.pool)
        .await
        .map_err(JobError::retryable)?;
        let negative_ids = sqlx::query_file_scalar!(
            "queries/recommendations/quality/select_negative_examples.sql",
            NEGATIVE_LIMIT
        )
        .fetch_all(&self.pool)
        .await
        .map_err(JobError::retryable)?;

        let mut labels = negative_ids
            .into_iter()
            .map(|id| (id, false))
            .collect::<HashMap<_, _>>();
        labels.extend(positive_ids.into_iter().map(|id| (id, true)));
        Ok(labels)
    }
}

pub(super) async fn save_if_changed(
    pool: &PgPool,
    trained: &TrainedModel,
) -> JobResult<Option<i64>> {
    if let Some(latest) = store::latest(pool).await?
        && !changed_meaningfully(&latest.model, &trained.model)
    {
        return Ok(None);
    }
    store::save(pool, trained).await.map(Some)
}

pub(super) fn changed_meaningfully(previous: &QualityModel, next: &QualityModel) -> bool {
    previous
        .means
        .iter()
        .zip(&next.means)
        .chain(previous.scales.iter().zip(&next.scales))
        .chain(previous.weights.iter().zip(&next.weights))
        .chain(std::iter::once((&previous.intercept, &next.intercept)))
        .any(|(before, after)| (after - before).abs() > MEANINGFUL_CHANGE * before.abs().max(1.0))
}
