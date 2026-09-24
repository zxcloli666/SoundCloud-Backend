use sqlx::PgPool;
use tracing::warn;

use crate::queue::{JobError, JobResult};

use super::features::FEATURE_COUNT;
use super::model::{QualityModel, TrainedModel};

pub struct StoredModel {
    pub version: i64,
    pub model: QualityModel,
}

pub async fn latest(pool: &PgPool) -> JobResult<Option<StoredModel>> {
    let Some(row) = sqlx::query_file!("queries/recommendations/quality/select_latest_model.sql")
        .fetch_optional(pool)
        .await
        .map_err(JobError::retryable)?
    else {
        return Ok(None);
    };
    let model = match (
        fixed(row.feature_means),
        fixed(row.feature_scales),
        fixed(row.weights),
    ) {
        (Some(means), Some(scales), Some(weights)) => QualityModel {
            means,
            scales,
            weights,
            intercept: row.intercept,
        },
        _ => {
            warn!(
                version = row.version,
                "stored quality model has the wrong shape; scoring with the fallback"
            );
            return Ok(None);
        }
    };
    if !model.is_usable() {
        warn!(
            version = row.version,
            "stored quality model is not usable; scoring with the fallback"
        );
        return Ok(None);
    }
    Ok(Some(StoredModel {
        version: row.version,
        model,
    }))
}

pub async fn save(pool: &PgPool, trained: &TrainedModel) -> JobResult<i64> {
    let examples = i32::try_from(trained.examples).map_err(JobError::permanent)?;
    let positives = i32::try_from(trained.positives).map_err(JobError::permanent)?;
    sqlx::query_file_scalar!(
        "queries/recommendations/quality/insert_model.sql",
        &trained.model.means[..],
        &trained.model.scales[..],
        &trained.model.weights[..],
        trained.model.intercept,
        examples,
        positives,
        trained.accuracy as f32
    )
    .fetch_one(pool)
    .await
    .map_err(JobError::retryable)
}

fn fixed(values: Vec<f32>) -> Option<[f32; FEATURE_COUNT]> {
    <[f32; FEATURE_COUNT]>::try_from(values).ok()
}
