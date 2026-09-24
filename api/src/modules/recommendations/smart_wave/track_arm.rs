use std::collections::{HashMap, HashSet};

use futures::{StreamExt, stream};
use qdrant_client::qdrant::{
    Filter, RecommendPointsBuilder, RecommendStrategy, Value as QdrantValue,
    point_id::PointIdOptions,
};
use tracing::debug;

use crate::modules::recommendations::service::RecommendationsService;
use crate::qdrant::collections;

const MERT_WEIGHT: f32 = 0.5;
const CLAP_WEIGHT: f32 = 0.3;
const LYRICS_WEIGHT: f32 = 0.2;
const PER_COLLECTION: u64 = 30;
const SEED_CONCURRENCY: usize = 4;

#[derive(Debug, Clone)]
pub struct TrackArmCandidate {
    pub sc_track_id: u64,
    pub score: f32,
}

struct RawCandidate {
    sc_track_id: u64,
    score: f32,
    payload: HashMap<String, QdrantValue>,
}

struct SeedCandidates {
    position: usize,
    recency_weight: f32,
    mert: Vec<RawCandidate>,
    clap: Vec<RawCandidate>,
    lyrics: Vec<RawCandidate>,
}

impl SeedCandidates {
    fn candidates(&self) -> impl Iterator<Item = &RawCandidate> {
        self.mert
            .iter()
            .chain(self.clap.iter())
            .chain(self.lyrics.iter())
    }
}

pub async fn recommend_from_many(
    svc: &RecommendationsService,
    seeds: &[u64],
    negative_ids: &[u64],
    filter: Option<&Filter>,
    limit: usize,
) -> Vec<TrackArmCandidate> {
    if seeds.is_empty() || limit == 0 {
        return Vec::new();
    }

    let mut input_ids = Vec::with_capacity(seeds.len() + negative_ids.len());
    input_ids.extend_from_slice(seeds);
    input_ids.extend_from_slice(negative_ids);
    input_ids.sort_unstable();
    input_ids.dedup();

    let (mert_vectors, clap_vectors, lyrics_vectors) = tokio::join!(
        svc.retrieve_vectors(collections::TRACKS_MERT, &input_ids),
        svc.retrieve_vectors(collections::TRACKS_CLAP, &input_ids),
        svc.retrieve_vectors(collections::TRACKS_LYRICS, &input_ids),
    );

    let mut per_seed = stream::iter(seeds.iter().copied().enumerate())
        .map(|(position, seed_track_id)| {
            let mert_vectors = &mert_vectors;
            let clap_vectors = &clap_vectors;
            let lyrics_vectors = &lyrics_vectors;
            async move {
                let (mert, clap, lyrics) = tokio::join!(
                    recommend_one(
                        svc,
                        collections::TRACKS_MERT,
                        seed_track_id,
                        negative_ids,
                        mert_vectors,
                        filter,
                    ),
                    recommend_one(
                        svc,
                        collections::TRACKS_CLAP,
                        seed_track_id,
                        negative_ids,
                        clap_vectors,
                        filter,
                    ),
                    recommend_one(
                        svc,
                        collections::TRACKS_LYRICS,
                        seed_track_id,
                        negative_ids,
                        lyrics_vectors,
                        filter,
                    ),
                );
                SeedCandidates {
                    position,
                    recency_weight: recency_factor(position, seeds.len()),
                    mert,
                    clap,
                    lyrics,
                }
            }
        })
        .buffer_unordered(SEED_CONCURRENCY)
        .collect::<Vec<_>>()
        .await;

    let mut candidate_ids = per_seed
        .iter()
        .flat_map(SeedCandidates::candidates)
        .map(|candidate| candidate.sc_track_id.to_string())
        .collect::<Vec<_>>();
    candidate_ids.sort_unstable();
    candidate_ids.dedup();

    let mut lyrics_candidate_ids = per_seed
        .iter()
        .flat_map(|seed| seed.lyrics.iter())
        .map(|candidate| candidate.sc_track_id.to_string())
        .collect::<Vec<_>>();
    lyrics_candidate_ids.sort_unstable();
    lyrics_candidate_ids.dedup();

    let (public_ids, lyrics_eligibility) = tokio::join!(
        svc.public_track_ids(&candidate_ids),
        svc.lyrics_vector_eligibility(&lyrics_candidate_ids),
    );
    let excluded_ids = seeds.iter().copied().collect::<HashSet<_>>();
    let per_seed_limit = limit.saturating_mul(2);

    per_seed.sort_unstable_by_key(|seed| seed.position);
    let weighted = per_seed.into_iter().map(|seed| {
        let mert = eligible_scores(seed.mert, &excluded_ids, &public_ids, |_, _| true);
        let clap = eligible_scores(seed.clap, &excluded_ids, &public_ids, |_, _| true);
        let lyrics = eligible_scores(
            seed.lyrics,
            &excluded_ids,
            &public_ids,
            |track_id, payload| lyrics_eligibility.matches(track_id, payload),
        );
        let candidates = blend_arms(mert, clap, lyrics, per_seed_limit);
        (candidates, seed.recency_weight)
    });

    merge_weighted(weighted, &excluded_ids, limit)
}

fn recency_factor(position: usize, total: usize) -> f32 {
    if total <= 1 {
        return 1.0;
    }
    let progress = position as f32 / (total - 1) as f32;
    (1.0 - 0.7 * progress).clamp(0.3, 1.0)
}

async fn recommend_one(
    svc: &RecommendationsService,
    collection: &str,
    seed_track_id: u64,
    negative_ids: &[u64],
    vectors: &HashMap<String, Vec<f32>>,
    filter: Option<&Filter>,
) -> Vec<RawCandidate> {
    let seed_id = seed_track_id.to_string();
    let Some(seed_vector) = vectors.get(&seed_id) else {
        return Vec::new();
    };
    let negative_vectors = negative_ids
        .iter()
        .filter(|negative_id| **negative_id != seed_track_id)
        .filter_map(|negative_id| vectors.get(&negative_id.to_string()).map(Vec::as_slice))
        .collect::<Vec<_>>();
    let request = recommend_request(
        collection,
        seed_vector,
        &negative_vectors,
        filter,
        collection == collections::TRACKS_LYRICS,
    );
    let raw = match Box::pin(svc.qdrant.raw().recommend(request)).await {
        Ok(response) => response.result,
        Err(error) => {
            debug!(collection, %error, "track arm Qdrant recommend failed");
            return Vec::new();
        }
    };

    raw.into_iter()
        .filter_map(|point| {
            let point_id = point.id?.point_id_options?;
            let PointIdOptions::Num(sc_track_id) = point_id else {
                return None;
            };
            Some(RawCandidate {
                sc_track_id,
                score: point.score,
                payload: point.payload,
            })
        })
        .collect()
}

fn recommend_request(
    collection: &str,
    seed_vector: &[f32],
    negative_vectors: &[&[f32]],
    filter: Option<&Filter>,
    with_payload: bool,
) -> RecommendPointsBuilder {
    let mut request = RecommendPointsBuilder::new(collection.to_owned(), PER_COLLECTION)
        .with_payload(with_payload)
        .strategy(RecommendStrategy::BestScore)
        .add_positive(seed_vector.to_vec());
    for vector in negative_vectors {
        request = request.add_negative(vector.to_vec());
    }
    if let Some(filter) = filter {
        request = request.filter(filter.clone());
    }
    request
}

fn eligible_scores(
    candidates: Vec<RawCandidate>,
    excluded_ids: &HashSet<u64>,
    public_ids: &HashSet<String>,
    is_active: impl Fn(&str, &HashMap<String, QdrantValue>) -> bool,
) -> Vec<(u64, f32)> {
    let scored = candidates
        .into_iter()
        .filter_map(|candidate| {
            if excluded_ids.contains(&candidate.sc_track_id) {
                return None;
            }
            let track_id = candidate.sc_track_id.to_string();
            if !public_ids.contains(&track_id) {
                return None;
            }
            if !is_active(&track_id, &candidate.payload) {
                return None;
            }
            Some((candidate.sc_track_id, candidate.score))
        })
        .collect();
    z_normalize(scored)
}

fn z_normalize(mut scored: Vec<(u64, f32)>) -> Vec<(u64, f32)> {
    let count = scored.len();
    if count < 2 {
        return scored.into_iter().map(|(id, _)| (id, 1.0)).collect();
    }
    let mean = scored.iter().map(|(_, score)| *score).sum::<f32>() / count as f32;
    let variance = scored
        .iter()
        .map(|(_, score)| (*score - mean).powi(2))
        .sum::<f32>()
        / count as f32;
    let standard_deviation = variance.sqrt().max(1e-6);
    for (_, score) in &mut scored {
        *score = (*score - mean) / standard_deviation;
    }
    scored
}

fn blend_arms(
    mert: Vec<(u64, f32)>,
    clap: Vec<(u64, f32)>,
    lyrics: Vec<(u64, f32)>,
    limit: usize,
) -> Vec<TrackArmCandidate> {
    let mut blended = HashMap::new();
    blend_in(&mut blended, &mert, MERT_WEIGHT);
    blend_in(&mut blended, &clap, CLAP_WEIGHT);
    blend_in(&mut blended, &lyrics, LYRICS_WEIGHT);
    rank(blended, limit)
}

fn merge_weighted(
    per_seed: impl IntoIterator<Item = (Vec<TrackArmCandidate>, f32)>,
    excluded_ids: &HashSet<u64>,
    limit: usize,
) -> Vec<TrackArmCandidate> {
    let mut accumulated = HashMap::new();
    for (candidates, weight) in per_seed {
        for candidate in candidates {
            if excluded_ids.contains(&candidate.sc_track_id) {
                continue;
            }
            *accumulated.entry(candidate.sc_track_id).or_insert(0.0) += candidate.score * weight;
        }
    }
    rank(accumulated, limit)
}

fn rank(scores: HashMap<u64, f32>, limit: usize) -> Vec<TrackArmCandidate> {
    let mut candidates = scores
        .into_iter()
        .map(|(sc_track_id, score)| TrackArmCandidate { sc_track_id, score })
        .collect::<Vec<_>>();
    candidates.sort_unstable_by(|left, right| right.score.total_cmp(&left.score));
    candidates.truncate(limit);
    candidates
}

fn blend_in(output: &mut HashMap<u64, f32>, scores: &[(u64, f32)], weight: f32) {
    for (track_id, score) in scores {
        *output.entry(*track_id).or_insert(0.0) += score * weight;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recommend_request_uses_only_validated_vectors() {
        let negative = vec![0.0, 1.0];
        let request =
            recommend_request("tracks", &[1.0, 0.0], &[negative.as_slice()], None, false).build();

        assert!(request.positive.is_empty());
        assert!(request.negative.is_empty());
        assert_eq!(request.positive_vectors.len(), 1);
        assert_eq!(request.negative_vectors.len(), 1);
    }

    #[test]
    fn candidates_from_multiple_seeds_accumulate() {
        let first = TrackArmCandidate {
            sc_track_id: 7,
            score: 1.0,
        };
        let second = TrackArmCandidate {
            sc_track_id: 7,
            score: 2.0,
        };
        let merged = merge_weighted(
            [(vec![first], 1.0), (vec![second], 0.5)],
            &HashSet::new(),
            10,
        );

        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].sc_track_id, 7);
        assert!((merged[0].score - 2.0).abs() < f32::EPSILON);
    }
}
