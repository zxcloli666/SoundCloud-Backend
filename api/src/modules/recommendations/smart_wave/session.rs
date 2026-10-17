use std::collections::{HashMap, HashSet};

use uuid::Uuid;

use super::cursor::WaveCursor;
use super::graph::Affinity;
use super::rank::TrackMeta;
use super::signals::UserSignals;
use super::{
    TasteCentroids, blend_centroids, centroids_mean, geomean, mean_centroid, opt_to_centroids,
};

const LIKED_ARTIST_AFFINITY: f32 = 0.8;
const SKIPPED_ARTIST_DAMP: f32 = 0.5;
const ARTIST_STRIKES_OUT: usize = 2;

#[derive(Debug, Default)]
pub struct SessionFeedback {
    pub positives: Vec<u64>,
    pub negatives: Vec<u64>,
    pub disliked: Vec<u64>,
}

impl SessionFeedback {
    pub fn from_signals(signals: &UserSignals, cursor: &WaveCursor) -> Self {
        let served: HashSet<u64> = cursor.seen_tracks.iter().copied().collect();
        let known: HashSet<u64> = cursor.known_positives.iter().copied().collect();
        let new_positive = |id: &u64| !known.contains(id);
        let mut decided: HashSet<u64> = HashSet::new();
        let mut session = Self::default();
        for id in served_ids(&signals.disliked_ids, &served) {
            if decided.insert(id) {
                session.negatives.push(id);
                session.disliked.push(id);
            }
        }
        let liked = signals.superliked.iter().chain(signals.fresh_likes.iter());
        for id in served_ids(liked, &served).into_iter().filter(new_positive) {
            if decided.insert(id) {
                session.positives.push(id);
            }
        }
        for id in served_ids(&signals.recent_skips, &served) {
            if decided.insert(id) {
                session.negatives.push(id);
            }
        }
        for id in served_ids(&signals.recent_played, &served)
            .into_iter()
            .filter(new_positive)
        {
            if decided.insert(id) {
                session.positives.push(id);
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
    ) {
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
        for (artist, count) in strikes {
            if count >= ARTIST_STRIKES_OUT && seed_artist != Some(artist) {
                affinity.remove(&artist);
                disliked_artists.insert(artist);
            } else if let Some(weight) = affinity.get_mut(&artist) {
                *weight *= SKIPPED_ARTIST_DAMP;
            }
        }
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

    pub(super) fn skipped_vectors(
        &self,
        vectors: [&HashMap<String, Vec<f32>>; 3],
    ) -> TasteCentroids {
        let [m, c, l] = vectors;
        TasteCentroids {
            m: vectors_by_id(&self.negatives, m).into_values().collect(),
            c: vectors_by_id(&self.negatives, c).into_values().collect(),
            l: vectors_by_id(&self.negatives, l).into_values().collect(),
        }
    }
}

pub fn positive_ids(signals: &UserSignals) -> HashSet<u64> {
    signals
        .superliked
        .iter()
        .chain(signals.fresh_likes.iter())
        .chain(signals.recent_played.iter())
        .filter_map(|id| id.parse::<u64>().ok())
        .collect()
}

pub fn away_from_skips(content: f32, near_skip: &[Option<f32>]) -> f32 {
    if !near_skip
        .iter()
        .flatten()
        .any(|similarity| *similarity > 0.0)
    {
        return content;
    }
    content * (content / geomean(near_skip)).min(1.0)
}

fn served_ids<'a>(ids: impl IntoIterator<Item = &'a String>, served: &HashSet<u64>) -> Vec<u64> {
    ids.into_iter()
        .filter_map(|id| id.parse::<u64>().ok())
        .filter(|id| served.contains(id))
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

    fn served(ids: &[u64]) -> WaveCursor {
        let mut cursor = WaveCursor::new(SeedKind::User, "17".into());
        for id in ids {
            cursor.mark_served(*id, None);
        }
        cursor
    }

    fn strings(ids: &[u64]) -> Vec<String> {
        ids.iter().map(u64::to_string).collect()
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
            fresh_likes: strings(&[1, 90]),
            recent_skips: strings(&[2, 91]),
            recent_played: strings(&[3]),
            ..UserSignals::default()
        };

        let session = SessionFeedback::from_signals(&signals, &served(&[1, 2, 3, 4]));

        assert_eq!(session.positives, vec![1, 3]);
        assert_eq!(session.negatives, vec![2]);
        assert!(session.disliked.is_empty());
    }

    #[test]
    fn a_like_or_listen_from_before_the_track_was_served_is_not_session_feedback() {
        let signals = UserSignals {
            fresh_likes: strings(&[1, 2]),
            recent_skips: strings(&[1, 4]),
            recent_played: strings(&[3]),
            ..UserSignals::default()
        };
        let mut cursor = served(&[1, 2, 3]);
        cursor.mark_known_positive(1);
        cursor.mark_known_positive(3);

        let session = SessionFeedback::from_signals(&signals, &cursor);

        assert_eq!(session.positives, vec![2]);
        assert_eq!(session.negatives, vec![1]);
        assert_eq!(positive_ids(&signals), HashSet::from([1, 2, 3]));
    }

    #[test]
    fn a_dislike_beats_a_like_and_a_like_beats_a_skip() {
        let signals = UserSignals {
            disliked_ids: strings(&[1]),
            fresh_likes: strings(&[1, 2]),
            recent_skips: strings(&[2, 3]),
            recent_played: strings(&[3]),
            ..UserSignals::default()
        };

        let session = SessionFeedback::from_signals(&signals, &served(&[1, 2, 3]));

        assert_eq!(session.negatives, vec![1, 3]);
        assert_eq!(session.disliked, vec![1]);
        assert_eq!(session.positives, vec![2]);
    }

    #[test]
    fn a_liked_artist_rises_and_a_skipped_one_sinks() {
        let session = SessionFeedback {
            positives: vec![1],
            negatives: vec![2],
            disliked: Vec::new(),
        };
        let mut affinity: Affinity = HashMap::from([(Uuid::from_u128(20), 0.6)]);
        let mut disliked = HashSet::new();

        session.steer_artists(
            &meta(&[(1, 10), (2, 20)]),
            None,
            &mut affinity,
            &mut disliked,
        );

        assert!((affinity[&Uuid::from_u128(10)] - LIKED_ARTIST_AFFINITY).abs() < 1e-6);
        assert!((affinity[&Uuid::from_u128(20)] - 0.3).abs() < 1e-6);
        assert!(disliked.is_empty());
    }

    #[test]
    fn two_skips_or_one_dislike_take_an_artist_out_unless_you_liked_it_too() {
        let session = SessionFeedback {
            positives: vec![5],
            negatives: vec![1, 2, 3, 4],
            disliked: vec![3],
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

        session.steer_artists(
            &meta(&[(1, 10), (2, 10)]),
            Some(seed_artist),
            &mut affinity,
            &mut disliked,
        );

        assert!(disliked.is_empty());
        assert!((affinity[&seed_artist] - SKIPPED_ARTIST_DAMP).abs() < 1e-6);
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
    fn a_track_closer_to_a_skip_than_to_the_taste_sinks() {
        let near = away_from_skips(0.8, &[Some(0.95), Some(0.95), None]);
        let far = away_from_skips(0.8, &[Some(0.5), Some(0.6), None]);
        let unknown = away_from_skips(0.8, &[None, None, None]);

        assert!(near < 0.7, "near a skip kept {near}");
        assert!((far - 0.8).abs() < 1e-6);
        assert!((unknown - 0.8).abs() < 1e-6);
    }
}
