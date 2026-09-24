use crate::modules::centroids::cosine;

pub fn greedy_pick<F>(pool_vecs: &[Vec<f32>], limit: usize, mut score: F) -> Vec<usize>
where
    F: FnMut(usize, &[usize], &[Vec<f32>]) -> f32,
{
    let want = limit.min(pool_vecs.len());
    if want == 0 {
        return Vec::new();
    }

    let mut selected: Vec<usize> = Vec::with_capacity(want);
    let mut taken = vec![false; pool_vecs.len()];

    while selected.len() < want {
        let mut best_idx = usize::MAX;
        let mut best_val = f32::NEG_INFINITY;
        for (i, &t) in taken.iter().enumerate() {
            if t {
                continue;
            }
            let s = score(i, &selected, pool_vecs);
            if s > best_val {
                best_val = s;
                best_idx = i;
            }
        }
        if best_idx == usize::MAX {
            break;
        }
        taken[best_idx] = true;
        selected.push(best_idx);
    }
    selected
}

pub fn max_cosine_to_selected(cand: usize, selected: &[usize], pool_vecs: &[Vec<f32>]) -> f32 {
    let mut m: f32 = 0.0;
    for &si in selected {
        let v = cosine(&pool_vecs[cand], &pool_vecs[si]);
        if v > m {
            m = v;
        }
    }
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    fn axes() -> Vec<Vec<f32>> {
        vec![
            vec![1.0, 0.0, 0.0],
            vec![0.99, 0.14, 0.0],
            vec![0.0, 1.0, 0.0],
            vec![0.0, 0.0, 1.0],
        ]
    }

    #[test]
    fn a_pick_never_repeats_a_candidate_even_when_one_always_scores_best() {
        let picks = greedy_pick(&axes(), 3, |_cand, _selected, _pool| 1.0);

        assert_eq!(picks.len(), 3);
        let mut sorted = picks.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), 3, "a candidate must not be picked twice");
    }

    #[test]
    fn a_limit_above_the_pool_returns_the_whole_pool_and_no_more() {
        let picks = greedy_pick(&axes(), 99, |cand, _selected, _pool| cand as f32);

        assert_eq!(picks.len(), 4);
    }

    #[test]
    fn an_empty_pool_and_a_zero_limit_both_pick_nothing() {
        assert!(greedy_pick(&[], 5, |_, _, _| 1.0).is_empty());
        assert!(greedy_pick(&axes(), 0, |_, _, _| 1.0).is_empty());
    }

    #[test]
    fn a_pool_where_nothing_is_admissible_stops_instead_of_spinning() {
        let picks = greedy_pick(&axes(), 4, |_cand, _selected, _pool| f32::NEG_INFINITY);

        assert!(
            picks.is_empty(),
            "a score that rules everything out must end the walk, not loop"
        );
    }

    #[test]
    fn diversity_beats_a_near_duplicate_of_what_is_already_picked() {
        let pool = axes();
        let relevance = [1.0_f32, 0.99, 0.5, 0.4];

        let picks = greedy_pick(&pool, 2, |cand, selected, pool_vecs| {
            relevance[cand] + 1.0 * (1.0 - max_cosine_to_selected(cand, selected, pool_vecs))
        });

        assert_eq!(picks[0], 0, "the most relevant candidate opens the list");
        assert_eq!(
            picks[1], 2,
            "with diversity weighted the twin of the first pick must lose to a new direction"
        );
    }

    #[test]
    fn nothing_selected_means_nothing_to_be_similar_to() {
        assert_eq!(max_cosine_to_selected(0, &[], &axes()), 0.0);
    }

    #[test]
    fn the_similarity_reported_is_the_closest_neighbour_not_the_last() {
        let pool = axes();

        let near = max_cosine_to_selected(1, &[0, 2], &pool);
        let far = max_cosine_to_selected(3, &[0, 2], &pool);

        assert!(
            near > 0.9,
            "candidate 1 twins candidate 0, which is not the last of the selected, and must still \
             read as similar, saw {near}"
        );
        assert!(far < 0.01, "an orthogonal candidate must read as new");
    }
}
