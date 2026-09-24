use chrono::{DateTime, Datelike, Timelike, Utc};
use sqlx::PgPool;

use crate::error::AppResult;
use crate::modules::centroids::normalize;
use crate::qdrant::collections;

use super::service::RecommendationsService;

const SESSION_WINDOW_HOURS: i64 = 2;
const SESSION_MIN_TRACKS: usize = 5;
const HOUR_WINDOW: u32 = 1;
const HOUR_LOOKBACK_WEEKS: i64 = 4;

pub struct SessionContext {
    pub centroid: Vec<f32>,
}

pub struct HourContext {
    pub centroid: Vec<f32>,
}

impl RecommendationsService {
    pub async fn detect_current_session(
        &self,
        sc_user_id: &str,
    ) -> AppResult<Option<SessionContext>> {
        let ids = recent_played_ids(&self.pg, sc_user_id, SESSION_WINDOW_HOURS, 80).await?;
        if ids.len() < SESSION_MIN_TRACKS {
            return Ok(None);
        }
        let numeric: Vec<u64> = ids.iter().filter_map(|s| s.parse::<u64>().ok()).collect();
        if numeric.is_empty() {
            return Ok(None);
        }
        let vec_map = self
            .retrieve_vectors(collections::TRACKS_MERT, &numeric)
            .await;
        let mut points: Vec<Vec<f32>> = numeric
            .iter()
            .filter_map(|n| vec_map.get(&n.to_string()).cloned())
            .collect();
        if points.len() < SESSION_MIN_TRACKS {
            return Ok(None);
        }
        let dim = points[0].len();
        let mut acc = vec![0f32; dim];
        let count = points.len() as f32;
        for v in points.drain(..) {
            for (i, x) in v.into_iter().enumerate() {
                if i < dim {
                    acc[i] += x;
                }
            }
        }
        for x in acc.iter_mut() {
            *x /= count;
        }
        normalize(&mut acc);
        Ok(Some(SessionContext { centroid: acc }))
    }

    pub async fn hour_context(
        &self,
        sc_user_id: &str,
        now: DateTime<Utc>,
    ) -> AppResult<Option<HourContext>> {
        let hour = now.hour();
        let dow = now.weekday().num_days_from_monday() as i32;
        let user_ids = crate::common::sc_ids::user_id_variants(sc_user_id);
        let ids: Vec<String> = sqlx::query_file_scalar!(
            "queries/recommendations/sessions/hour_context_ids.sql",
            &user_ids,
            HOUR_LOOKBACK_WEEKS as i32,
            hour as i32,
            HOUR_WINDOW as i32,
            dow,
        )
        .fetch_all(&self.pg)
        .await
        .unwrap_or_default();
        if ids.len() < 5 {
            return Ok(None);
        }
        let numeric: Vec<u64> = ids.iter().filter_map(|s| s.parse::<u64>().ok()).collect();
        if numeric.is_empty() {
            return Ok(None);
        }
        let vec_map = self
            .retrieve_vectors(collections::TRACKS_MERT, &numeric)
            .await;
        let mut acc: Option<Vec<f32>> = None;
        let mut count = 0usize;
        for n in &numeric {
            if let Some(v) = vec_map.get(&n.to_string()) {
                match acc.as_mut() {
                    Some(a) => {
                        let dim = a.len().min(v.len());
                        for i in 0..dim {
                            a[i] += v[i];
                        }
                    }
                    None => acc = Some(v.clone()),
                }
                count += 1;
            }
        }
        let mut a = match acc {
            Some(a) => a,
            None => return Ok(None),
        };
        if count == 0 {
            return Ok(None);
        }
        let inv = 1.0 / count as f32;
        for x in a.iter_mut() {
            *x *= inv;
        }
        normalize(&mut a);
        Ok(Some(HourContext { centroid: a }))
    }
}

async fn recent_played_ids(
    pg: &PgPool,
    sc_user_id: &str,
    hours: i64,
    limit: i64,
) -> AppResult<Vec<String>> {
    let user_ids = crate::common::sc_ids::user_id_variants(sc_user_id);
    let rows = sqlx::query_file_scalar!(
        "queries/recommendations/sessions/recent_played_ids.sql",
        &user_ids,
        hours as i32,
        limit,
    )
    .fetch_all(pg)
    .await?;
    Ok(rows)
}

pub fn mix_centroids(
    base: Option<&[f32]>,
    session: Option<&[f32]>,
    hour: Option<&[f32]>,
) -> Option<Vec<f32>> {
    const W_BASE: f32 = 0.6;
    const W_SESSION: f32 = 0.25;
    const W_HOUR: f32 = 0.15;

    let any = base.is_some() || session.is_some() || hour.is_some();
    if !any {
        return None;
    }
    let dim = base
        .map(|v| v.len())
        .or_else(|| session.map(|v| v.len()))
        .or_else(|| hour.map(|v| v.len()))?;
    let mut acc = vec![0f32; dim];
    let mut total_w = 0f32;
    let mut add = |v: Option<&[f32]>, w: f32| {
        if let Some(v) = v {
            let n = dim.min(v.len());
            for i in 0..n {
                acc[i] += v[i] * w;
            }
            total_w += w;
        }
    };
    add(base, W_BASE);
    add(session, W_SESSION);
    add(hour, W_HOUR);
    if total_w <= 0.0 {
        return None;
    }
    for x in acc.iter_mut() {
        *x /= total_w;
    }
    normalize(&mut acc);
    Some(acc)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modules::centroids::cosine;

    const BASE: [f32; 3] = [1.0, 0.0, 0.0];
    const SESSION: [f32; 3] = [0.0, 1.0, 0.0];
    const HOUR: [f32; 3] = [0.0, 0.0, 1.0];

    #[test]
    fn with_nothing_to_mix_there_is_no_centroid() {
        assert!(mix_centroids(None, None, None).is_none());
    }

    #[test]
    fn a_single_source_comes_back_as_itself() {
        for source in [BASE, SESSION, HOUR] {
            let only_base = mix_centroids(Some(&source), None, None).expect("one source is enough");
            let only_session =
                mix_centroids(None, Some(&source), None).expect("one source is enough");
            let only_hour = mix_centroids(None, None, Some(&source)).expect("one source is enough");

            for mixed in [only_base, only_session, only_hour] {
                assert!((cosine(&mixed, &source) - 1.0).abs() < 1e-5);
            }
        }
    }

    #[test]
    fn the_long_taste_outweighs_the_session_and_the_session_outweighs_the_hour() {
        let mixed = mix_centroids(Some(&BASE), Some(&SESSION), Some(&HOUR)).expect("three sources");

        assert!(
            mixed[0] > mixed[1] && mixed[1] > mixed[2],
            "the weights must keep their order, saw {mixed:?}"
        );
    }

    #[test]
    fn a_missing_source_does_not_shrink_what_is_left() {
        let mixed = mix_centroids(Some(&BASE), None, Some(&HOUR)).expect("two sources");

        assert!(
            (mixed[0] / mixed[2] - 4.0).abs() < 1e-4,
            "the surviving sources must keep their ratio 0.6 to 0.15, saw {mixed:?}"
        );
        assert!(
            (mixed.iter().map(|x| x * x).sum::<f32>() - 1.0).abs() < 1e-5,
            "the mix is always a unit vector"
        );
    }

    #[test]
    fn a_shorter_source_contributes_what_it_has_instead_of_panicking() {
        let short = [1.0f32];
        let mixed = mix_centroids(Some(&BASE), Some(&short), None).expect("two sources");

        assert_eq!(mixed.len(), 3);
        assert!(mixed.iter().all(|x| !x.is_nan()));
    }

    #[test]
    fn centroids_of_zeros_do_not_turn_into_nans() {
        let zeros = [0.0f32, 0.0, 0.0];
        let mixed = mix_centroids(Some(&zeros), Some(&zeros), None).expect("two sources");

        assert_eq!(mixed, vec![0.0, 0.0, 0.0]);
    }
}
