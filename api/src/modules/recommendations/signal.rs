use std::collections::{HashMap, HashSet};

use sqlx::PgPool;

use crate::error::AppResult;

const IMPLICIT_POSITIVE: &str = "full_play";
const NEGATIVE_TYPES: &[&str] = &["dislike", "skip"];

const DECAY_HALF_LIFE_DAYS: f32 = 90.0;
const POSITIVE_LIMIT: i64 = 80;
const NEGATIVE_LIMIT: i64 = 200;
const PLAYED_LIMIT: i64 = 300;
const STRONG_POSITIVE_MIN: usize = 8;
const IMPLICIT_POSITIVE_MIN: usize = 12;
const PLAYED_FALLBACK_MIN: usize = 20;

#[derive(Debug, Clone)]
pub struct WeightedTrack {
    pub sc_track_id: String,
    pub weight: f32,
}

#[derive(Debug, Default)]
pub struct UserSignals {
    pub strong_positives: Vec<WeightedTrack>,
    pub implicit_positives: Vec<WeightedTrack>,
    pub played: Vec<String>,
    pub negatives: Vec<WeightedTrack>,
    pub disliked_ids: Vec<String>,
}

impl UserSignals {
    pub fn best_seed_kind(&self) -> SeedKind {
        if self.strong_positives.len() >= STRONG_POSITIVE_MIN {
            SeedKind::Strong
        } else if self.implicit_positives.len() >= IMPLICIT_POSITIVE_MIN {
            SeedKind::Implicit
        } else if self.played.len() >= PLAYED_FALLBACK_MIN {
            SeedKind::Played
        } else {
            SeedKind::ColdStart
        }
    }

    pub fn positive_seed(&self) -> Vec<WeightedTrack> {
        match self.best_seed_kind() {
            SeedKind::Strong => self.strong_positives.clone(),
            SeedKind::Implicit => {
                let mut out: Vec<WeightedTrack> =
                    Vec::with_capacity(self.strong_positives.len() + self.implicit_positives.len());
                let mut placed: HashMap<&str, usize> = HashMap::new();
                for track in self
                    .strong_positives
                    .iter()
                    .chain(self.implicit_positives.iter())
                {
                    match placed.get(track.sc_track_id.as_str()) {
                        Some(&at) => {
                            let kept: &mut WeightedTrack = &mut out[at];
                            if track.weight > kept.weight {
                                kept.weight = track.weight;
                            }
                        }
                        None => {
                            placed.insert(track.sc_track_id.as_str(), out.len());
                            out.push(track.clone());
                        }
                    }
                }
                out
            }
            SeedKind::Played => self
                .played
                .iter()
                .map(|id| WeightedTrack {
                    sc_track_id: id.clone(),
                    weight: 0.1,
                })
                .collect(),
            SeedKind::ColdStart => Vec::new(),
        }
    }

    pub fn has_any_signal(&self) -> bool {
        !self.strong_positives.is_empty()
            || !self.implicit_positives.is_empty()
            || !self.played.is_empty()
            || !self.negatives.is_empty()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeedKind {
    Strong,
    Implicit,
    Played,
    ColdStart,
}

pub async fn load_user_signals(pg: &PgPool, sc_user_id: &str) -> AppResult<UserSignals> {
    let variants = crate::common::sc_ids::user_id_variants(sc_user_id);
    let disliked_ids: Vec<String> =
        sqlx::query_file_scalar!("queries/recommendations/signal/disliked_ids.sql", &variants)
            .fetch_all(pg)
            .await
            .unwrap_or_default();
    let disliked_set: HashSet<String> = disliked_ids.iter().cloned().collect();

    let strong_positives = load_strong_positives(pg, sc_user_id, &disliked_set).await;

    let event_filter: &[&str] = &[IMPLICIT_POSITIVE, "skip", "dislike"];
    let event_rows = sqlx::query_file!(
        "queries/recommendations/signal/event_rows.sql",
        &variants,
        event_filter as &[&str],
        NEGATIVE_LIMIT + PLAYED_LIMIT
    )
    .fetch_all(pg)
    .await
    .unwrap_or_default();

    let mut implicit_positives: Vec<WeightedTrack> = Vec::new();
    let mut played: Vec<String> = Vec::new();
    let mut negatives: Vec<WeightedTrack> = Vec::new();
    let mut seen_played: HashSet<String> = strong_positives
        .iter()
        .map(|w| w.sc_track_id.clone())
        .collect();

    for r in event_rows {
        if disliked_set.contains(&r.sc_track_id) {
            continue;
        }
        let decay = decay_factor(r.age_days);
        if r.event_type == IMPLICIT_POSITIVE && implicit_positives.len() < POSITIVE_LIMIT as usize {
            let multiplier = match r.position_pct {
                Some(p) if p >= 0.85 => 1.0,
                Some(p) if p >= 0.65 => 0.6,
                _ => 0.3,
            };
            implicit_positives.push(WeightedTrack {
                sc_track_id: r.sc_track_id.clone(),
                weight: (r.weight.max(0.0) as f32) * decay * multiplier,
            });
        }
        if NEGATIVE_TYPES.contains(&r.event_type.as_str())
            && negatives.len() < NEGATIVE_LIMIT as usize
        {
            negatives.push(WeightedTrack {
                sc_track_id: r.sc_track_id.clone(),
                weight: (r.weight.min(0.0).abs() as f32) * decay,
            });
        }
        if played.len() < PLAYED_LIMIT as usize && seen_played.insert(r.sc_track_id.clone()) {
            played.push(r.sc_track_id);
        }
    }

    for w in &strong_positives {
        if played.len() < PLAYED_LIMIT as usize && seen_played.insert(w.sc_track_id.clone()) {
            played.push(w.sc_track_id.clone());
        }
    }

    for id in &disliked_ids {
        if negatives.iter().all(|n| &n.sc_track_id != id) {
            negatives.push(WeightedTrack {
                sc_track_id: id.clone(),
                weight: 1.0,
            });
        }
    }

    Ok(UserSignals {
        strong_positives,
        implicit_positives,
        played,
        negatives,
        disliked_ids,
    })
}

async fn load_strong_positives(
    pg: &PgPool,
    sc_user_id: &str,
    disliked: &HashSet<String>,
) -> Vec<WeightedTrack> {
    let mut out: Vec<WeightedTrack> = Vec::with_capacity(POSITIVE_LIMIT as usize);
    let mut seen: HashSet<String> = HashSet::new();
    let variants = crate::common::sc_ids::user_id_variants(sc_user_id);

    let event_likes = sqlx::query_file!(
        "queries/recommendations/signal/event_likes.sql",
        &variants,
        POSITIVE_LIMIT
    )
    .fetch_all(pg)
    .await
    .unwrap_or_default();

    for r in event_likes {
        if disliked.contains(&r.sc_track_id) || !seen.insert(r.sc_track_id.clone()) {
            continue;
        }
        let decay = decay_factor(r.age_days);
        out.push(WeightedTrack {
            sc_track_id: r.sc_track_id,
            weight: (r.weight.max(0.0) as f32) * decay,
        });
        if out.len() >= POSITIVE_LIMIT as usize {
            return out;
        }
    }

    if out.len() >= STRONG_POSITIVE_MIN {
        return out;
    }

    let need_more = (POSITIVE_LIMIT as usize).saturating_sub(out.len());
    let mirror_likes = sqlx::query_file!(
        "queries/recommendations/signal/mirror_likes.sql",
        &variants,
        need_more as i64
    )
    .fetch_all(pg)
    .await
    .unwrap_or_default();

    for r in mirror_likes {
        if disliked.contains(&r.sc_track_id) || !seen.insert(r.sc_track_id.clone()) {
            continue;
        }
        out.push(WeightedTrack {
            sc_track_id: r.sc_track_id,
            weight: decay_factor(r.age_days),
        });
        if out.len() >= POSITIVE_LIMIT as usize {
            break;
        }
    }

    out
}

fn decay_factor(age_days: f32) -> f32 {
    if age_days.is_nan() || age_days < 0.0 {
        return 1.0;
    }
    (-age_days * std::f32::consts::LN_2 / DECAY_HALF_LIFE_DAYS).exp()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track(id: &str, weight: f32) -> WeightedTrack {
        WeightedTrack {
            sc_track_id: id.to_owned(),
            weight,
        }
    }

    fn many(prefix: &str, count: usize) -> Vec<WeightedTrack> {
        (0..count)
            .map(|index| track(&format!("{prefix}{index}"), 1.0))
            .collect()
    }

    fn ids(seed: &[WeightedTrack]) -> Vec<String> {
        seed.iter().map(|w| w.sc_track_id.clone()).collect()
    }

    #[test]
    fn a_user_with_nothing_is_a_cold_start_and_seeds_nothing() {
        let signals = UserSignals::default();

        assert_eq!(signals.best_seed_kind(), SeedKind::ColdStart);
        assert!(signals.positive_seed().is_empty());
        assert!(!signals.has_any_signal());
    }

    #[test]
    fn each_seed_kind_starts_exactly_at_its_threshold() {
        let strong = UserSignals {
            strong_positives: many("like", STRONG_POSITIVE_MIN),
            ..UserSignals::default()
        };
        assert_eq!(strong.best_seed_kind(), SeedKind::Strong);

        let almost_strong = UserSignals {
            strong_positives: many("like", STRONG_POSITIVE_MIN - 1),
            implicit_positives: many("play", IMPLICIT_POSITIVE_MIN),
            ..UserSignals::default()
        };
        assert_eq!(almost_strong.best_seed_kind(), SeedKind::Implicit);

        let only_played = UserSignals {
            implicit_positives: many("play", IMPLICIT_POSITIVE_MIN - 1),
            played: (0..PLAYED_FALLBACK_MIN).map(|i| format!("p{i}")).collect(),
            ..UserSignals::default()
        };
        assert_eq!(only_played.best_seed_kind(), SeedKind::Played);

        let too_thin = UserSignals {
            played: (0..PLAYED_FALLBACK_MIN - 1)
                .map(|i| format!("p{i}"))
                .collect(),
            ..UserSignals::default()
        };
        assert_eq!(too_thin.best_seed_kind(), SeedKind::ColdStart);
    }

    #[test]
    fn a_strong_taste_is_seeded_from_likes_alone() {
        let signals = UserSignals {
            strong_positives: many("like", STRONG_POSITIVE_MIN),
            implicit_positives: many("play", IMPLICIT_POSITIVE_MIN),
            ..UserSignals::default()
        };

        let seed = signals.positive_seed();

        assert_eq!(seed.len(), STRONG_POSITIVE_MIN);
        assert!(
            seed.iter().all(|w| w.sc_track_id.starts_with("like")),
            "with enough likes the full plays must not dilute the taste"
        );
    }

    #[test]
    fn a_thin_taste_is_topped_up_with_full_plays() {
        let signals = UserSignals {
            strong_positives: many("like", 2),
            implicit_positives: many("play", IMPLICIT_POSITIVE_MIN),
            ..UserSignals::default()
        };

        let seed = signals.positive_seed();

        assert_eq!(seed.len(), 2 + IMPLICIT_POSITIVE_MIN);
        assert_eq!(&ids(&seed)[..2], &["like0".to_owned(), "like1".to_owned()]);
    }

    #[test]
    fn a_track_both_liked_and_played_through_enters_the_seed_once() {
        let signals = UserSignals {
            strong_positives: vec![track("42", 1.0)],
            implicit_positives: many("play", IMPLICIT_POSITIVE_MIN - 1)
                .into_iter()
                .chain([track("42", 0.6)])
                .collect(),
            ..UserSignals::default()
        };

        let seed = signals.positive_seed();

        assert_eq!(
            ids(&seed).iter().filter(|id| *id == "42").count(),
            1,
            "one track is one point: counted twice it drags the centroid and the k-means split \
             towards itself"
        );
        assert_eq!(
            seed.iter()
                .find(|w| w.sc_track_id == "42")
                .map(|w| w.weight),
            Some(1.0),
            "the stronger signal decides the weight, so a like is not diluted by a full play"
        );
        assert_eq!(seed.len(), IMPLICIT_POSITIVE_MIN);
    }

    #[test]
    fn a_played_only_seed_carries_a_deliberately_faint_weight() {
        let signals = UserSignals {
            played: (0..PLAYED_FALLBACK_MIN).map(|i| format!("p{i}")).collect(),
            ..UserSignals::default()
        };

        let seed = signals.positive_seed();

        assert_eq!(seed.len(), PLAYED_FALLBACK_MIN);
        assert!(seed.iter().all(|w| w.weight < 0.2));
    }

    #[test]
    fn taste_halves_every_ninety_days() {
        assert!((decay_factor(0.0) - 1.0).abs() < 1e-6);
        assert!((decay_factor(DECAY_HALF_LIFE_DAYS) - 0.5).abs() < 1e-5);
        assert!((decay_factor(DECAY_HALF_LIFE_DAYS * 2.0) - 0.25).abs() < 1e-5);
        assert!(decay_factor(3650.0) > 0.0, "decay must not reach zero");
    }

    #[test]
    fn a_broken_timestamp_does_not_erase_the_signal() {
        assert_eq!(decay_factor(f32::NAN), 1.0);
        assert_eq!(decay_factor(-5.0), 1.0);
    }

    #[test]
    fn a_single_dislike_is_already_a_signal() {
        let signals = UserSignals {
            negatives: vec![track("42", 1.0)],
            ..UserSignals::default()
        };

        assert!(signals.has_any_signal());
        assert_eq!(signals.best_seed_kind(), SeedKind::ColdStart);
    }
}
