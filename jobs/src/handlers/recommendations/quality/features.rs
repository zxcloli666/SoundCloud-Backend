use std::collections::HashMap;

use backend_contracts::vector_store::{TRACKS_CLAP, TRACKS_MERT};
use sqlx::PgPool;

use crate::qdrant::QdrantProvisioner;
use crate::queue::{JobError, JobResult};

pub const FEATURE_COUNT: usize = 10;

#[derive(Clone, Debug)]
pub struct TrackMeta {
    plays: i64,
    likes: i64,
    duration_ms: i64,
    title: String,
    has_genre: bool,
    is_preview: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct QualityFeatures([f32; FEATURE_COUNT]);

impl QualityFeatures {
    pub fn build(meta: &TrackMeta, mert_stats: (f32, f32), clap_stats: (f32, f32)) -> Self {
        Self([
            mert_stats.0,
            mert_stats.1,
            clap_stats.0,
            clap_stats.1,
            ((meta.plays as f64).ln_1p() as f32) / 16.0,
            ((meta.likes as f64).ln_1p() as f32) / 12.0,
            (meta.duration_ms as f32) / 60_000.0,
            (meta.title.len() as f32 / 100.0).min(1.0),
            if meta.has_genre { 1.0 } else { 0.0 },
            if meta.is_preview { 1.0 } else { 0.0 },
        ])
    }

    #[cfg(test)]
    pub fn from_values(values: [f32; FEATURE_COUNT]) -> Self {
        Self(values)
    }

    pub fn as_array(&self) -> &[f32; FEATURE_COUNT] {
        &self.0
    }

    pub fn fallback_score(&self) -> f32 {
        let [
            _,
            _,
            _,
            _,
            log_plays,
            log_likes,
            duration_minutes,
            _,
            _,
            is_preview,
        ] = self.0;
        let score = 0.4 * (log_plays * 16.0 / 6.0).tanh()
            + 0.3 * (log_likes * 12.0 / 4.0).tanh()
            + 0.2 * (1.0 - (((duration_minutes - 3.5).abs()) / 5.0).min(1.0))
            + 0.1 * (1.0 - is_preview);
        score.clamp(0.0, 1.0)
    }
}

pub async fn load_features(
    pool: &PgPool,
    qdrant: &QdrantProvisioner,
    ids: Vec<String>,
) -> JobResult<Vec<(String, QualityFeatures)>> {
    let metadata = load_track_meta(pool, &ids).await?;
    let numeric_ids = ids
        .iter()
        .filter_map(|id| id.parse::<u64>().ok())
        .collect::<Vec<_>>();
    let (mert, clap) = tokio::join!(
        qdrant.retrieve_vectors(TRACKS_MERT, &numeric_ids),
        qdrant.retrieve_vectors(TRACKS_CLAP, &numeric_ids),
    );
    let mert = mert.map_err(JobError::retryable)?;
    let clap = clap.map_err(JobError::retryable)?;
    Ok(ids
        .into_iter()
        .filter_map(|id| {
            let features = QualityFeatures::build(
                metadata.get(&id)?,
                vector_stats(mert.get(&id)),
                vector_stats(clap.get(&id)),
            );
            Some((id, features))
        })
        .collect())
}

async fn load_track_meta(pool: &PgPool, ids: &[String]) -> JobResult<HashMap<String, TrackMeta>> {
    if ids.is_empty() {
        return Ok(HashMap::new());
    }
    let rows = sqlx::query_file!("queries/recommendations/quality/load_track_meta.sql", ids)
        .fetch_all(pool)
        .await
        .map_err(JobError::retryable)?;

    let mut metadata = HashMap::with_capacity(rows.len());
    for row in rows {
        let lower_title = row.title.to_lowercase();
        metadata.insert(
            row.sc_track_id,
            TrackMeta {
                plays: row.play_count.unwrap_or(0),
                likes: row.likes_count.unwrap_or(0),
                duration_ms: i64::from(row.duration_ms),
                has_genre: row.genre.is_some_and(|genre| !genre.is_empty()),
                is_preview: lower_title.contains("preview") || lower_title.contains("teaser"),
                title: row.title,
            },
        );
    }
    Ok(metadata)
}

pub fn vector_stats(vector: Option<&Vec<f32>>) -> (f32, f32) {
    let Some(vector) = vector.filter(|values| !values.is_empty()) else {
        return (0.0, 0.0);
    };
    let count = vector.len() as f32;
    let mean = vector.iter().sum::<f32>() / count;
    let variance = vector
        .iter()
        .map(|value| (value - mean).powi(2))
        .sum::<f32>()
        / count;
    (mean, variance.sqrt())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fallback_score_is_finite_and_bounded() {
        let features = QualityFeatures::build(
            &TrackMeta {
                plays: 1_000,
                likes: 50,
                duration_ms: 210_000,
                title: "Track".to_owned(),
                has_genre: true,
                is_preview: false,
            },
            (0.1, 0.2),
            (0.3, 0.4),
        );

        let score = features.fallback_score();
        assert!(score.is_finite());
        assert!((0.0..=1.0).contains(&score));
        assert_eq!(features.as_array().len(), FEATURE_COUNT);
    }

    #[test]
    fn empty_vector_has_zero_statistics() {
        assert_eq!(vector_stats(None), (0.0, 0.0));
        assert_eq!(vector_stats(Some(&Vec::new())), (0.0, 0.0));
    }
}
