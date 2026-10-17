use std::cmp::Reverse;
use std::collections::{HashMap, HashSet};

use uuid::Uuid;

use super::cursor::WaveCursor;
use super::graph::Affinity;
use super::rank::TrackMeta;
use super::signals::{Signal, UserSignals};
use super::{
    TasteCentroids, blend_centroids, centroids_mean, geomean, mean_centroid, opt_to_centroids,
};
use crate::modules::centroids::normalize;

const LIKED_ARTIST_AFFINITY: f32 = 0.8;
pub const SKIPPED_ARTIST_DAMP: f32 = 0.5;
const ARTIST_STRIKES_OUT: usize = 2;
const SKIPS_TO_AVOID: usize = 8;

#[derive(Debug, Default)]
pub struct SessionFeedback {
    pub positives: Vec<u64>,
    pub negatives: Vec<u64>,
    pub disliked: Vec<u64>,
    pub lately: Vec<u64>,
}

impl SessionFeedback {
    pub fn from_signals(signals: &UserSignals, cursor: &WaveCursor, lately_since: i64) -> Self {
        let since = cursor.history_until.unwrap_or(i64::MAX);
        let served: HashSet<u64> = cursor.seen_tracks.iter().copied().collect();
        let mut decided: HashSet<u64> = HashSet::new();
        let mut session = Self::default();
        for (id, _) in in_session(&signals.disliked_ids, since, &served) {
            if decided.insert(id) {
                session.negatives.push(id);
                session.disliked.push(id);
            }
        }
        let liked = [
            in_session(&signals.superliked, since, &served),
            in_session(&signals.fresh_likes, since, &served),
        ];
        for (id, _) in liked.concat() {
            if decided.insert(id) {
                session.positives.push(id);
            }
        }
        let skips = in_session(&signals.recent_skips, since, &served)
            .into_iter()
            .map(|(id, at)| (id, at, false));
        let full_plays = in_session(&signals.recent_played, since, &served)
            .into_iter()
            .map(|(id, at)| (id, at, true));
        let mut listens: Vec<(u64, i64, bool)> = skips.chain(full_plays).collect();
        listens.sort_by_key(|(_, at, _)| Reverse(*at));
        for (id, _, played) in listens {
            if !decided.insert(id) {
                continue;
            }
            if played {
                session.positives.push(id);
            } else {
                session.negatives.push(id);
            }
        }
        let mut kept: HashMap<u64, i64> = HashMap::new();
        let positive = signals
            .fresh_likes
            .iter()
            .chain(&signals.superliked)
            .chain(&signals.recent_played);
        for signal in positive {
            if let Ok(id) = signal.id.parse::<u64>() {
                let at = kept.entry(id).or_insert(signal.at);
                *at = (*at).max(signal.at);
            }
        }
        let mut lately: Vec<(u64, i64)> = signals
            .disliked_ids
            .iter()
            .chain(&signals.recent_skips)
            .filter(|signal| signal.at > lately_since)
            .filter_map(|signal| Some((signal.id.parse::<u64>().ok()?, signal.at)))
            .filter(|(id, at)| kept.get(id).is_none_or(|kept_at| kept_at < at))
            .collect();
        lately.sort_by_key(|(_, at)| Reverse(*at));
        for (id, _) in lately {
            if decided.insert(id) {
                session.lately.push(id);
            }
        }
        session
    }

    pub fn track_ids(&self) -> Vec<u64> {
        self.positives
            .iter()
            .chain(self.negatives.iter())
            .copied()
            .collect()
    }

    pub fn steer_artists(
        &self,
        meta: &HashMap<u64, TrackMeta>,
        seed_artist: Option<Uuid>,
        affinity: &mut Affinity,
        disliked_artists: &mut HashSet<Uuid>,
    ) -> HashSet<Uuid> {
        let artist_of = |id: &u64| meta.get(id).and_then(|m| m.primary_artist);
        let liked: HashSet<Uuid> = self.positives.iter().filter_map(artist_of).collect();
        let mut strikes: HashMap<Uuid, usize> = HashMap::new();
        for artist in self
            .negatives
            .iter()
            .chain(self.disliked.iter())
            .filter_map(artist_of)
        {
            if !liked.contains(&artist) {
                *strikes.entry(artist).or_insert(0) += 1;
            }
        }
        for artist in liked {
            let weight = affinity.entry(artist).or_insert(0.0);
            *weight = weight.max(LIKED_ARTIST_AFFINITY);
        }
        let mut sunk = HashSet::new();
        for (artist, count) in strikes {
            if count >= ARTIST_STRIKES_OUT && seed_artist != Some(artist) {
                affinity.remove(&artist);
                disliked_artists.insert(artist);
            } else {
                sunk.insert(artist);
            }
        }
        sunk
    }

    pub(super) fn pull_taste(
        &self,
        taste: TasteCentroids,
        vectors: [&HashMap<String, Vec<f32>>; 3],
        boosted: &HashSet<u64>,
        weight: f32,
    ) -> TasteCentroids {
        if self.positives.is_empty() {
            return taste;
        }
        let [m, c, l] = vectors;
        let pull = |centroids: &[Vec<f32>], vectors: &HashMap<String, Vec<f32>>| {
            let liked = vectors_by_id(&self.positives, vectors);
            opt_to_centroids(blend_centroids(
                mean_centroid(&liked, boosted),
                centroids_mean(centroids),
                weight,
            ))
        };
        TasteCentroids {
            m: pull(&taste.m, m),
            c: pull(&taste.c, c),
            l: pull(&taste.l, l),
        }
    }

    pub fn avoided(&self) -> Vec<u64> {
        self.negatives
            .iter()
            .chain(&self.lately)
            .take(SKIPS_TO_AVOID)
            .copied()
            .collect()
    }

    pub(super) fn skipped_vectors(
        &self,
        vectors: [&HashMap<String, Vec<f32>>; 3],
    ) -> TasteCentroids {
        let avoided = self.avoided();
        let unit = |vectors: &HashMap<String, Vec<f32>>| -> Vec<Vec<f32>> {
            vectors_by_id(&avoided, vectors)
                .into_values()
                .map(|mut vector| {
                    normalize(&mut vector);
                    vector
                })
                .collect()
        };
        let [m, c, l] = vectors;
        TasteCentroids {
            m: unit(m),
            c: unit(c),
            l: unit(l),
        }
    }
}

pub fn nearest_skip(skipped: &[Vec<f32>], vector: Option<&Vec<f32>>) -> Option<f32> {
    if skipped.is_empty() {
        return None;
    }
    let vector = vector?;
    let norm = dot(vector, vector).sqrt();
    if norm == 0.0 {
        return None;
    }
    skipped
        .iter()
        .map(|skip| dot(vector, skip) / norm)
        .reduce(f32::max)
}

pub fn skip_damp(taste: &[Option<f32>], near_skip: &[Option<f32>]) -> f32 {
    let ratios: Vec<Option<f32>> = taste
        .iter()
        .zip(near_skip)
        .map(|pair| match pair {
            (Some(taste), Some(skip)) if *taste > 0.0 && *skip > 0.0 => Some(taste / skip),
            _ => None,
        })
        .collect();
    geomean(&ratios).min(1.0)
}

fn dot(a: &[f32], b: &[f32]) -> f32 {
    let n = a.len().min(b.len());
    let (a_chunks, a_rest) = a[..n].as_chunks::<8>();
    let (b_chunks, b_rest) = b[..n].as_chunks::<8>();
    let mut lanes = [0f32; 8];
    for (a, b) in a_chunks.iter().zip(b_chunks) {
        for ((lane, x), y) in lanes.iter_mut().zip(a).zip(b) {
            *lane += x * y;
        }
    }
    let rest: f32 = a_rest.iter().zip(b_rest).map(|(x, y)| x * y).sum();
    lanes.iter().sum::<f32>() + rest
}

fn in_session(signals: &[Signal], since: i64, served: &HashSet<u64>) -> Vec<(u64, i64)> {
    signals
        .iter()
        .filter(|signal| signal.at > since)
        .filter_map(|signal| Some((signal.id.parse::<u64>().ok()?, signal.at)))
        .filter(|(id, _)| served.contains(id))
        .collect()
}

fn vectors_by_id(ids: &[u64], vectors: &HashMap<String, Vec<f32>>) -> HashMap<String, Vec<f32>> {
    ids.iter()
        .filter_map(|id| {
            let key = id.to_string();
            vectors.get(&key).map(|vector| (key, vector.clone()))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modules::recommendations::smart_wave::cursor::SeedKind;

    const HISTORY: i64 = 1_000;
    const EARLIER: i64 = HISTORY - 600;
    const LATER: i64 = HISTORY + 60;
    const LATELY_SINCE: i64 = HISTORY - 3_600;

    fn served(ids: &[u64]) -> WaveCursor {
        let mut cursor = WaveCursor::new(SeedKind::User, "17".into());
        cursor.history_until = Some(HISTORY);
        for id in ids {
            cursor.mark_served(*id, None);
        }
        cursor
    }

    fn at(time: i64, ids: &[u64]) -> Vec<Signal> {
        ids.iter()
            .map(|id| Signal {
                id: id.to_string(),
                at: time,
            })
            .collect()
    }

    fn meta(pairs: &[(u64, u128)]) -> HashMap<u64, TrackMeta> {
        pairs
            .iter()
            .map(|(id, artist)| {
                (
                    *id,
                    TrackMeta {
                        primary_artist: Some(Uuid::from_u128(*artist)),
                        storage_ok: true,
                    },
                )
            })
            .collect()
    }

    #[test]
    fn only_what_this_wave_served_counts_as_session_feedback() {
        let signals = UserSignals {
            fresh_likes: at(LATER, &[1, 90]),
            recent_skips: at(LATER, &[2, 91]),
            recent_played: at(LATER, &[3]),
            ..UserSignals::default()
        };

        let session = SessionFeedback::from_signals(&signals, &served(&[1, 2, 3, 4]), LATELY_SINCE);

        assert_eq!(session.positives, vec![1, 3]);
        assert_eq!(session.negatives, vec![2]);
        assert!(session.disliked.is_empty());
    }

    #[test]
    fn what_happened_before_the_wave_started_is_not_session_feedback() {
        let signals = UserSignals {
            fresh_likes: [at(EARLIER, &[1]), at(LATER, &[2])].concat(),
            recent_skips: at(EARLIER, &[1, 4]),
            recent_played: at(EARLIER, &[3]),
            ..UserSignals::default()
        };

        let session = SessionFeedback::from_signals(&signals, &served(&[1, 2, 3, 4]), LATELY_SINCE);

        assert_eq!(session.positives, vec![2]);
        assert!(
            session.negatives.is_empty(),
            "an old skip of a served track counted: {:?}",
            session.negatives
        );
        assert_eq!(session.lately, vec![4]);
    }

    #[test]
    fn skips_and_dislikes_outside_the_session_only_sink_by_sound() {
        let signals = UserSignals {
            disliked_ids: at(LATER + 10, &[93]),
            recent_skips: [
                at(LATER, &[90, 92]),
                at(LATELY_SINCE - 1, &[91]),
                at(LATER, &[1]),
            ]
            .concat(),
            recent_played: at(LATER + 5, &[92]),
            ..UserSignals::default()
        };

        let session = SessionFeedback::from_signals(&signals, &served(&[1]), LATELY_SINCE);

        assert_eq!(session.negatives, vec![1]);
        assert_eq!(session.lately, vec![93, 90]);
        assert_eq!(session.avoided(), vec![1, 93, 90]);
        let mut steered: Affinity = HashMap::new();
        let mut disliked = HashSet::new();
        let sunk = session.steer_artists(
            &meta(&[(1, 10), (90, 20), (93, 30)]),
            None,
            &mut steered,
            &mut disliked,
        );
        assert_eq!(sunk, HashSet::from([Uuid::from_u128(10)]));
        assert!(disliked.is_empty());
    }

    #[test]
    fn a_wave_without_a_history_mark_has_no_session_yet() {
        let signals = UserSignals {
            recent_skips: at(LATER, &[1]),
            ..UserSignals::default()
        };
        let mut cursor = served(&[1]);
        cursor.history_until = None;

        let session = SessionFeedback::from_signals(&signals, &cursor, LATELY_SINCE);

        assert!(session.negatives.is_empty());
    }

    #[test]
    fn the_latest_listen_of_a_track_decides_between_skip_and_full_play() {
        let signals = UserSignals {
            recent_skips: [
                at(EARLIER, &[1]),
                at(LATER + 20, &[2]),
                at(LATER + 10, &[3]),
            ]
            .concat(),
            recent_played: [at(LATER, &[1]), at(LATER + 10, &[2]), at(LATER + 20, &[3])].concat(),
            ..UserSignals::default()
        };

        let session = SessionFeedback::from_signals(&signals, &served(&[1, 2, 3]), LATELY_SINCE);

        assert_eq!(session.positives, vec![3, 1]);
        assert_eq!(session.negatives, vec![2]);
    }

    #[test]
    fn a_superlike_in_the_wave_counts_on_a_track_liked_long_before() {
        let signals = UserSignals {
            fresh_likes: at(EARLIER, &[1]),
            superliked: at(LATER, &[1]),
            ..UserSignals::default()
        };

        let session = SessionFeedback::from_signals(&signals, &served(&[1]), LATELY_SINCE);

        assert_eq!(session.positives, vec![1]);
    }

    #[test]
    fn a_dislike_beats_a_like_and_a_like_beats_a_skip() {
        let signals = UserSignals {
            disliked_ids: at(LATER, &[1]),
            fresh_likes: at(LATER, &[1, 2]),
            recent_skips: at(LATER, &[2, 3]),
            recent_played: at(LATER - 1, &[3]),
            ..UserSignals::default()
        };

        let session = SessionFeedback::from_signals(&signals, &served(&[1, 2, 3]), LATELY_SINCE);

        assert_eq!(session.negatives, vec![1, 3]);
        assert_eq!(session.disliked, vec![1]);
        assert_eq!(session.positives, vec![2]);
    }

    #[test]
    fn a_liked_artist_rises_and_a_skipped_one_sinks() {
        let session = SessionFeedback {
            positives: vec![1],
            negatives: vec![2],
            ..SessionFeedback::default()
        };
        let mut affinity: Affinity = HashMap::from([(Uuid::from_u128(20), 0.6)]);
        let mut disliked = HashSet::new();

        let sunk = session.steer_artists(
            &meta(&[(1, 10), (2, 20), (3, 30)]),
            None,
            &mut affinity,
            &mut disliked,
        );

        assert!((affinity[&Uuid::from_u128(10)] - LIKED_ARTIST_AFFINITY).abs() < 1e-6);
        assert_eq!(sunk, HashSet::from([Uuid::from_u128(20)]));
        assert!(disliked.is_empty());
    }

    #[test]
    fn two_skips_or_one_dislike_take_an_artist_out_unless_you_liked_it_too() {
        let session = SessionFeedback {
            positives: vec![5],
            negatives: vec![1, 2, 3, 4],
            disliked: vec![3],
            ..SessionFeedback::default()
        };
        let mut affinity: Affinity = HashMap::from([
            (Uuid::from_u128(10), 1.0),
            (Uuid::from_u128(20), 1.0),
            (Uuid::from_u128(30), 1.0),
        ]);
        let mut disliked = HashSet::new();

        session.steer_artists(
            &meta(&[(1, 10), (2, 10), (3, 20), (4, 30), (5, 30)]),
            None,
            &mut affinity,
            &mut disliked,
        );

        assert_eq!(
            disliked,
            HashSet::from([Uuid::from_u128(10), Uuid::from_u128(20)])
        );
        assert!(!affinity.contains_key(&Uuid::from_u128(10)));
        assert!((affinity[&Uuid::from_u128(30)] - 1.0).abs() < 1e-6);
    }

    #[test]
    fn an_artist_wave_never_skips_its_own_artist_out_only_down() {
        let session = SessionFeedback {
            negatives: vec![1, 2],
            ..SessionFeedback::default()
        };
        let seed_artist = Uuid::from_u128(10);
        let mut affinity: Affinity = HashMap::from([(seed_artist, 1.0)]);
        let mut disliked = HashSet::new();

        let sunk = session.steer_artists(
            &meta(&[(1, 10), (2, 10)]),
            Some(seed_artist),
            &mut affinity,
            &mut disliked,
        );

        assert!(disliked.is_empty());
        assert_eq!(sunk, HashSet::from([seed_artist]));
    }

    #[test]
    fn a_liked_track_pulls_the_taste_toward_itself() {
        let session = SessionFeedback {
            positives: vec![1],
            ..SessionFeedback::default()
        };
        let liked = HashMap::from([("1".to_string(), vec![0.0, 1.0])]);
        let taste = TasteCentroids {
            m: vec![vec![1.0, 0.0]],
            c: Vec::new(),
            l: Vec::new(),
        };

        let pulled = session.pull_taste(taste, [&liked, &liked, &liked], &HashSet::new(), 0.4);

        assert_eq!(pulled.m, vec![vec![0.6, 0.4]]);
        assert_eq!(pulled.c, vec![vec![0.0, 1.0]]);
    }

    #[test]
    fn only_the_latest_skips_are_compared_with_the_next_candidates() {
        let session = SessionFeedback {
            negatives: (1..=40).collect(),
            ..SessionFeedback::default()
        };
        let vectors: HashMap<String, Vec<f32>> = (1..=40)
            .map(|id| {
                let vector = if id <= SKIPS_TO_AVOID as u64 {
                    vec![3.0, 0.0]
                } else {
                    vec![0.0, 3.0]
                };
                (id.to_string(), vector)
            })
            .collect();

        let skipped = session.skipped_vectors([&vectors, &vectors, &HashMap::new()]);

        assert_eq!(skipped.m.len(), SKIPS_TO_AVOID);
        assert!(skipped.m.iter().all(|vector| vector[..] == [1.0, 0.0]));
        assert!(skipped.l.is_empty());
    }

    #[test]
    fn the_nearest_skip_is_a_cosine_over_any_length() {
        let mut along = vec![0.0f32; 11];
        along[10] = 1.0;
        let mut across = vec![0.0f32; 11];
        across[3] = 1.0;
        let mut candidate = vec![0.0f32; 11];
        candidate[10] = 4.0;
        candidate[3] = 3.0;

        let nearest = nearest_skip(&[along, across.clone()], Some(&candidate));

        assert!(nearest.is_some_and(|similarity| (similarity - 0.8).abs() < 1e-6));
        assert_eq!(nearest_skip(&[], Some(&candidate)), None);
        assert_eq!(nearest_skip(&[across], None), None);
    }

    #[test]
    fn a_track_closer_to_a_skip_than_to_the_taste_sinks() {
        let taste = [Some(0.8), Some(0.8), None];

        let near = skip_damp(&taste, &[Some(0.95), Some(0.95), None]);
        let far = skip_damp(&taste, &[Some(0.5), Some(0.6), None]);
        let unknown = skip_damp(&taste, &[None, None, None]);

        assert!(near < 0.9, "near a skip kept {near}");
        assert_eq!(far, 1.0);
        assert_eq!(unknown, 1.0);
    }

    #[test]
    fn a_skip_counts_only_in_the_spaces_the_taste_has_too() {
        let damp = skip_damp(&[Some(0.8), None, None], &[Some(0.7), None, Some(0.99)]);

        assert_eq!(damp, 1.0);
    }
}
