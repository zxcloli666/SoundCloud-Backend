use std::collections::{HashMap, HashSet};

use crate::modules::centroids::cosine;
use crate::qdrant::collections;

use super::mmr::{greedy_pick, max_cosine_to_selected};
use super::service::util::value_to_u64;
use super::service::{RecommendResult, RecommendationsService};

pub struct RerankOptions {
    pub limit: usize,
    pub diversity: f32,
    pub novelty: f32,
    pub serendipity: f32,
    pub anti_centroid: Option<Vec<f32>>,
    pub recent_artists: HashSet<String>,
    pub user_centroid: Option<Vec<f32>>,
}

impl Default for RerankOptions {
    fn default() -> Self {
        Self {
            limit: 12,
            diversity: 0.35,
            novelty: 0.15,
            serendipity: 0.10,
            anti_centroid: None,
            recent_artists: HashSet::new(),
            user_centroid: None,
        }
    }
}

fn work_limit(available: usize, limit: usize) -> usize {
    available.min(limit.saturating_mul(4))
}

impl RecommendationsService {
    pub async fn rerank_multi(
        &self,
        items: Vec<RecommendResult>,
        opts: RerankOptions,
    ) -> Vec<RecommendResult> {
        if items.is_empty() || opts.limit == 0 {
            return items;
        }
        let (head, tail) = items.split_at(work_limit(items.len(), opts.limit));
        let head_vec: Vec<RecommendResult> = head.to_vec();
        let tail_vec: Vec<RecommendResult> = tail.to_vec();

        let numeric_ids: Vec<u64> = head_vec
            .iter()
            .filter_map(|it| value_to_u64(&it.id))
            .collect();
        if numeric_ids.is_empty() {
            return [head_vec, tail_vec].concat();
        }
        let vec_map: HashMap<String, Vec<f32>> = self
            .retrieve_vectors(collections::TRACKS_MERT, &numeric_ids)
            .await;
        if vec_map.is_empty() {
            return [head_vec, tail_vec].concat();
        }

        let mut pool: Vec<(RecommendResult, Vec<f32>)> = head_vec
            .into_iter()
            .filter_map(|it| {
                let id = value_to_u64(&it.id)?;
                let v = vec_map.get(&id.to_string()).cloned()?;
                Some((it, v))
            })
            .collect();
        if pool.is_empty() {
            return tail_vec;
        }

        if let Some(anti) = opts.anti_centroid.as_deref() {
            pool.retain(|(_, v)| cosine(v, anti) < 0.85);
        }
        pool.sort_by(|a, b| {
            b.0.score
                .unwrap_or(0.0)
                .partial_cmp(&a.0.score.unwrap_or(0.0))
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        if pool.is_empty() {
            return tail_vec;
        }

        let pool_vecs: Vec<Vec<f32>> = pool.iter().map(|(_, v)| v.clone()).collect();
        let relevances: Vec<f32> = pool.iter().map(|(it, _)| it.score.unwrap_or(0.0)).collect();
        let novelty_flags: Vec<f32> = pool
            .iter()
            .map(|(it, _)| novelty_flag(it.artist.as_deref(), &opts.recent_artists))
            .collect();
        let user_centroid = opts.user_centroid.clone();
        let weights = RerankWeights {
            diversity: opts.diversity,
            novelty: opts.novelty,
            serendipity: opts.serendipity,
        };

        let picks = greedy_pick(&pool_vecs, opts.limit, |cand, selected, pool_vecs| {
            rerank_score(
                cand,
                selected,
                pool_vecs,
                &relevances,
                &novelty_flags,
                user_centroid.as_deref(),
                &weights,
            )
        });

        let mut taken = vec![false; pool.len()];
        let mut selected: Vec<RecommendResult> = Vec::with_capacity(picks.len());
        for idx in picks {
            taken[idx] = true;
            selected.push(pool[idx].0.clone());
        }
        let leftover: Vec<RecommendResult> = pool
            .into_iter()
            .enumerate()
            .filter_map(|(i, (it, _))| if taken[i] { None } else { Some(it) })
            .collect();

        [selected, leftover, tail_vec].concat()
    }
}

struct RerankWeights {
    diversity: f32,
    novelty: f32,
    serendipity: f32,
}

fn novelty_flag(artist: Option<&str>, recent: &HashSet<String>) -> f32 {
    match artist {
        Some(name) if recent.contains(&name.to_lowercase()) => 0.0,
        Some(_) => 1.0,
        None => 0.6,
    }
}

fn rerank_score(
    cand: usize,
    selected: &[usize],
    pool_vecs: &[Vec<f32>],
    relevances: &[f32],
    novelty_flags: &[f32],
    user_centroid: Option<&[f32]>,
    weights: &RerankWeights,
) -> f32 {
    let rel = relevances[cand];
    let diversity_term = 1.0 - max_cosine_to_selected(cand, selected, pool_vecs);
    let serendipity_term = match user_centroid {
        Some(uc) => (1.0 - cosine(&pool_vecs[cand], uc)).clamp(0.0, 1.0) * rel.max(0.0),
        None => 0.0,
    };
    rel + weights.diversity * diversity_term
        + weights.novelty * novelty_flags[cand]
        + weights.serendipity * serendipity_term
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pool_smaller_than_the_page_is_reranked_whole_instead_of_panicking() {
        for available in 0..6 {
            assert_eq!(
                work_limit(available, 6),
                available,
                "asking for six out of {available} must not reach past the end of the pool"
            );
        }
    }

    #[test]
    fn a_deep_pool_is_cut_to_four_pages() {
        assert_eq!(work_limit(1000, 6), 24);
        assert_eq!(work_limit(24, 6), 24);
        assert_eq!(work_limit(25, 6), 24);
    }

    #[test]
    fn an_absurd_page_size_does_not_overflow_into_a_tiny_window() {
        assert_eq!(work_limit(10, usize::MAX), 10);
    }

    fn recent(names: &[&str]) -> HashSet<String> {
        names.iter().map(|name| name.to_lowercase()).collect()
    }

    fn silent() -> RerankWeights {
        RerankWeights {
            diversity: 0.0,
            novelty: 0.0,
            serendipity: 0.0,
        }
    }

    fn axes() -> Vec<Vec<f32>> {
        vec![
            vec![1.0, 0.0, 0.0],
            vec![0.0, 1.0, 0.0],
            vec![-1.0, 0.0, 0.0],
        ]
    }

    #[test]
    fn an_artist_just_heard_carries_no_novelty_whatever_the_case() {
        let heard = recent(&["Boards of Canada"]);

        assert_eq!(novelty_flag(Some("BOARDS OF CANADA"), &heard), 0.0);
        assert_eq!(novelty_flag(Some("Aphex Twin"), &heard), 1.0);
        assert_eq!(
            novelty_flag(None, &heard),
            0.6,
            "an unknown artist is neither fresh nor stale"
        );
    }

    #[test]
    fn with_every_weight_at_zero_only_relevance_decides() {
        let pool = axes();
        let relevances = [0.9, 0.1, 0.5];
        let novelty = [1.0, 1.0, 1.0];

        for cand in 0..3 {
            let score = rerank_score(cand, &[], &pool, &relevances, &novelty, None, &silent());
            assert!((score - relevances[cand]).abs() < 1e-6);
        }
    }

    #[test]
    fn a_new_direction_scores_above_a_twin_of_what_is_already_picked() {
        let pool = axes();
        let relevances = [0.5, 0.5, 0.5];
        let novelty = [0.0, 0.0, 0.0];
        let weights = RerankWeights {
            diversity: 1.0,
            novelty: 0.0,
            serendipity: 0.0,
        };

        let twin = rerank_score(0, &[0], &pool, &relevances, &novelty, None, &weights);
        let fresh = rerank_score(1, &[0], &pool, &relevances, &novelty, None, &weights);

        assert!(fresh > twin, "saw fresh {fresh} against twin {twin}");
    }

    #[test]
    fn serendipity_is_nothing_without_a_taste_to_be_far_from() {
        let pool = axes();
        let relevances = [1.0, 1.0, 1.0];
        let novelty = [0.0, 0.0, 0.0];
        let weights = RerankWeights {
            diversity: 0.0,
            novelty: 0.0,
            serendipity: 1.0,
        };

        let blind = rerank_score(1, &[], &pool, &relevances, &novelty, None, &weights);

        assert!((blind - 1.0).abs() < 1e-6);
    }

    #[test]
    fn a_track_far_from_the_taste_gains_more_than_one_next_to_it() {
        let pool = axes();
        let relevances = [1.0, 1.0, 1.0];
        let novelty = [0.0, 0.0, 0.0];
        let weights = RerankWeights {
            diversity: 0.0,
            novelty: 0.0,
            serendipity: 1.0,
        };
        let taste = [1.0f32, 0.0, 0.0];

        let near = rerank_score(0, &[], &pool, &relevances, &novelty, Some(&taste), &weights);
        let far = rerank_score(1, &[], &pool, &relevances, &novelty, Some(&taste), &weights);

        assert!((near - 1.0).abs() < 1e-6, "the taste itself gains nothing");
        assert!(far > near);
    }

    #[test]
    fn the_opposite_of_the_taste_cannot_collect_a_double_bonus() {
        let pool = axes();
        let relevances = [1.0, 1.0, 1.0];
        let novelty = [0.0, 0.0, 0.0];
        let weights = RerankWeights {
            diversity: 0.0,
            novelty: 0.0,
            serendipity: 1.0,
        };
        let taste = [1.0f32, 0.0, 0.0];

        let orthogonal = rerank_score(1, &[], &pool, &relevances, &novelty, Some(&taste), &weights);
        let opposite = rerank_score(2, &[], &pool, &relevances, &novelty, Some(&taste), &weights);

        assert!(
            (opposite - orthogonal).abs() < 1e-6,
            "the bonus is clamped at one, otherwise the opposite of the taste wins twice over: \
             opposite {opposite}, orthogonal {orthogonal}"
        );
    }

    #[test]
    fn a_track_nobody_wants_is_not_promoted_for_being_strange() {
        let pool = axes();
        let novelty = [0.0, 0.0, 0.0];
        let weights = RerankWeights {
            diversity: 0.0,
            novelty: 0.0,
            serendipity: 1.0,
        };
        let taste = [1.0f32, 0.0, 0.0];

        let worthless = rerank_score(
            1,
            &[],
            &pool,
            &[0.0, 0.0, 0.0],
            &novelty,
            Some(&taste),
            &weights,
        );

        assert!(
            worthless.abs() < 1e-6,
            "serendipity is scaled by relevance, so a zero-score track gains nothing, saw {worthless}"
        );
    }

    #[test]
    fn a_negative_score_cannot_turn_serendipity_into_a_penalty() {
        let pool = axes();
        let novelty = [0.0, 0.0, 0.0];
        let weights = RerankWeights {
            diversity: 0.0,
            novelty: 0.0,
            serendipity: 1.0,
        };
        let taste = [1.0f32, 0.0, 0.0];

        let scored = rerank_score(
            1,
            &[],
            &pool,
            &[-2.0, -2.0, -2.0],
            &novelty,
            Some(&taste),
            &weights,
        );

        assert!((scored + 2.0).abs() < 1e-6, "saw {scored}");
    }
}
