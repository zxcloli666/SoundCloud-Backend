use std::collections::HashMap;

use crate::modules::centroids::{cosine, normalize};
use crate::qdrant::collections;

use super::service::RecommendationsService;
use super::signal::WeightedTrack;

const KMEANS_MAX_ITERS: usize = 12;
const KMEANS_TOL: f32 = 1e-3;

pub fn pick_k(seed_count: usize) -> usize {
    match seed_count {
        0..=7 => 1,
        8..=15 => 2,
        16..=40 => 3,
        _ => 4,
    }
}

pub struct TasteMode {
    pub centroid: Vec<f32>,
}

impl RecommendationsService {
    pub async fn build_taste_modes(&self, seeds: &[WeightedTrack]) -> Vec<TasteMode> {
        if seeds.is_empty() {
            return Vec::new();
        }
        let track_ids: Vec<u64> = seeds
            .iter()
            .filter_map(|s| s.sc_track_id.parse::<u64>().ok())
            .collect();
        if track_ids.is_empty() {
            return Vec::new();
        }
        let vec_map = self
            .retrieve_vectors(collections::TRACKS_MERT, &track_ids)
            .await;

        let points: Vec<(Vec<f32>, f32)> = seeds
            .iter()
            .filter_map(|s| {
                vec_map
                    .get(&s.sc_track_id)
                    .map(|v| (v.clone(), s.weight.max(0.05)))
            })
            .collect();
        if points.is_empty() {
            return Vec::new();
        }

        let k = pick_k(points.len()).min(points.len());
        if k == 1 {
            let mut centroid = weighted_mean(&points);
            normalize(&mut centroid);
            return vec![TasteMode { centroid }];
        }

        kmeans(&points, k)
    }
}

fn weighted_mean(points: &[(Vec<f32>, f32)]) -> Vec<f32> {
    let dim = points[0].0.len();
    let mut acc = vec![0f32; dim];
    let mut total_w = 0f32;
    for (v, w) in points {
        let n = dim.min(v.len());
        for i in 0..n {
            acc[i] += v[i] * w;
        }
        total_w += w;
    }
    if total_w > 0.0 {
        for x in acc.iter_mut() {
            *x /= total_w;
        }
    }
    acc
}

fn kmeans(points: &[(Vec<f32>, f32)], k: usize) -> Vec<TasteMode> {
    let mut centers: Vec<Vec<f32>> = seed_centers(points, k);
    let mut last_assignment = vec![usize::MAX; points.len()];

    for _ in 0..KMEANS_MAX_ITERS {
        let mut changed = 0usize;
        let mut groups: Vec<Vec<(Vec<f32>, f32)>> = vec![Vec::new(); k];

        for (idx, (v, w)) in points.iter().enumerate() {
            let mut best = 0usize;
            let mut best_sim = f32::NEG_INFINITY;
            for (ci, c) in centers.iter().enumerate() {
                let s = cosine(v, c);
                if s > best_sim {
                    best_sim = s;
                    best = ci;
                }
            }
            if last_assignment[idx] != best {
                changed += 1;
                last_assignment[idx] = best;
            }
            groups[best].push((v.clone(), *w));
        }

        let mut max_shift = 0f32;
        for (ci, group) in groups.iter().enumerate() {
            if group.is_empty() {
                continue;
            }
            let mut new_c = weighted_mean(group);
            normalize(&mut new_c);
            let shift = 1.0 - cosine(&centers[ci], &new_c);
            if shift > max_shift {
                max_shift = shift;
            }
            centers[ci] = new_c;
        }

        if changed == 0 || max_shift < KMEANS_TOL {
            break;
        }
    }

    let mut sizes = vec![0usize; k];
    for &a in last_assignment.iter() {
        if a < k {
            sizes[a] += 1;
        }
    }

    centers
        .into_iter()
        .enumerate()
        .filter(|(i, _)| sizes[*i] > 0)
        .map(|(_, c)| TasteMode { centroid: c })
        .collect()
}

fn seed_centers(points: &[(Vec<f32>, f32)], k: usize) -> Vec<Vec<f32>> {
    let mut centers: Vec<Vec<f32>> = Vec::with_capacity(k);
    let mut sorted: Vec<usize> = (0..points.len()).collect();
    sorted.sort_by(|a, b| {
        points[*b]
            .1
            .partial_cmp(&points[*a].1)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    centers.push(points[sorted[0]].0.clone());
    while centers.len() < k {
        let mut best_idx = 0usize;
        let mut best_dist = f32::NEG_INFINITY;
        for &idx in &sorted {
            let v = &points[idx].0;
            let mut min_sim = 1.0f32;
            for c in &centers {
                let s = cosine(v, c);
                if s < min_sim {
                    min_sim = s;
                }
            }
            let dist = 1.0 - min_sim;
            if dist > best_dist {
                best_dist = dist;
                best_idx = idx;
            }
        }
        centers.push(points[best_idx].0.clone());
    }
    for c in centers.iter_mut() {
        normalize(c);
    }
    centers
}

pub fn build_anti_centroid(
    neg_vec_map: &HashMap<String, Vec<f32>>,
    weights: &[(String, f32)],
) -> Option<Vec<f32>> {
    if weights.is_empty() {
        return None;
    }
    let mut points: Vec<(Vec<f32>, f32)> = Vec::new();
    for (id, w) in weights {
        if let Some(v) = neg_vec_map.get(id) {
            points.push((v.clone(), *w));
        }
    }
    if points.is_empty() {
        return None;
    }
    let mut acc = weighted_mean(&points);
    normalize(&mut acc);
    Some(acc)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn point(v: &[f32], w: f32) -> (Vec<f32>, f32) {
        (v.to_vec(), w)
    }

    #[test]
    fn the_number_of_modes_grows_in_steps_with_the_seed_count() {
        assert_eq!([pick_k(0), pick_k(7)], [1, 1]);
        assert_eq!([pick_k(8), pick_k(15)], [2, 2]);
        assert_eq!([pick_k(16), pick_k(40)], [3, 3]);
        assert_eq!([pick_k(41), pick_k(10_000)], [4, 4]);
    }

    #[test]
    fn a_heavier_seed_pulls_the_mean_towards_itself() {
        let mean = weighted_mean(&[point(&[1.0, 0.0], 3.0), point(&[0.0, 1.0], 1.0)]);

        assert!(
            mean[0] > mean[1],
            "weight must decide the direction, saw {mean:?}"
        );
        assert!((mean[0] - 0.75).abs() < 1e-5);
        assert!((mean[1] - 0.25).abs() < 1e-5);
    }

    #[test]
    fn weightless_seeds_give_a_zero_mean_and_not_a_nan() {
        let mean = weighted_mean(&[point(&[1.0, 2.0], 0.0), point(&[3.0, 4.0], 0.0)]);

        assert!(mean.iter().all(|x| !x.is_nan()), "saw {mean:?}");
        assert_eq!(mean, vec![0.0, 0.0]);
    }

    #[test]
    fn a_short_vector_contributes_what_it_has_instead_of_panicking() {
        let mean = weighted_mean(&[point(&[1.0, 1.0, 1.0], 1.0), point(&[1.0], 1.0)]);

        assert_eq!(mean.len(), 3);
        assert!((mean[0] - 1.0).abs() < 1e-5);
        assert!((mean[1] - 0.5).abs() < 1e-5);
    }

    #[test]
    fn two_clear_clusters_come_back_as_two_modes() {
        let mut points = Vec::new();
        for _ in 0..6 {
            points.push(point(&[1.0, 0.0, 0.0], 1.0));
        }
        for _ in 0..6 {
            points.push(point(&[0.0, 1.0, 0.0], 1.0));
        }

        let modes = kmeans(&points, 2);

        assert_eq!(modes.len(), 2);
        let mut best = [0f32, 0f32];
        for mode in &modes {
            best[0] = best[0].max(cosine(&mode.centroid, &[1.0, 0.0, 0.0]));
            best[1] = best[1].max(cosine(&mode.centroid, &[0.0, 1.0, 0.0]));
        }
        assert!(best[0] > 0.99 && best[1] > 0.99, "saw {best:?}");
    }

    #[test]
    fn seeds_that_are_all_the_same_taste_produce_one_mode_not_two_copies() {
        let points: Vec<(Vec<f32>, f32)> = (0..8).map(|_| point(&[1.0, 0.0, 0.0], 1.0)).collect();

        let modes = kmeans(&points, 2);

        assert_eq!(
            modes.len(),
            1,
            "an empty cluster must be dropped, not returned as a duplicate centroid"
        );
    }

    #[test]
    fn the_starting_centres_are_taken_far_apart() {
        let points = vec![
            point(&[1.0, 0.0, 0.0], 5.0),
            point(&[0.99, 0.14, 0.0], 4.0),
            point(&[0.0, 0.0, 1.0], 1.0),
        ];

        let centers = seed_centers(&points, 2);

        assert!((cosine(&centers[0], &[1.0, 0.0, 0.0]) - 1.0).abs() < 1e-5);
        assert!(
            cosine(&centers[1], &[0.0, 0.0, 1.0]) > 0.99,
            "the second centre must be the far point, not the twin of the first"
        );
    }

    #[test]
    fn an_anti_centroid_needs_both_a_weight_and_a_vector() {
        let mut map = HashMap::new();
        map.insert("a".to_owned(), vec![1.0, 0.0]);

        assert!(build_anti_centroid(&map, &[]).is_none());
        assert!(build_anti_centroid(&map, &[("b".to_owned(), 1.0)]).is_none());

        let built = build_anti_centroid(&map, &[("a".to_owned(), 1.0)]).expect("a is known");
        assert!((cosine(&built, &[1.0, 0.0]) - 1.0).abs() < 1e-5);
        assert!(
            (built[0] - 1.0).abs() < 1e-5,
            "the result must be unit length"
        );
    }
}
