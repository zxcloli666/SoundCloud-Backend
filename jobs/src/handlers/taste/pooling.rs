use std::collections::BTreeMap;

use anyhow::{Context, ensure};
use serde::Deserialize;

use super::history::{EventKind, Positives, TasteEvent};

const DEFAULT_WINDOW: usize = 200;
const MAX_WINDOW: usize = 10_000;
const SECONDS_PER_DAY: f64 = 86_400.0;

#[derive(Deserialize)]
struct PoolingSpec {
    w: BTreeMap<String, f64>,
    tau_days: f64,
    #[serde(default, alias = "max_events")]
    window: Option<usize>,
    #[serde(default)]
    strength: Strength,
    #[serde(default = "one_positive")]
    min_positives: usize,
}

fn one_positive() -> usize {
    1
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
enum Strength {
    #[default]
    TypeOnly,
    AbsWeight,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Pooling {
    weights: [Option<f32>; 6],
    tau_days: f64,
    window: usize,
    strength: Strength,
    min_positives: usize,
}

impl Pooling {
    pub(crate) fn from_json(value: &serde_json::Value) -> anyhow::Result<Self> {
        let spec = PoolingSpec::deserialize(value).context("taste pooling is malformed")?;
        ensure!(
            spec.tau_days.is_finite() && spec.tau_days > 0.0,
            "taste pooling tau_days must be a positive number"
        );
        let window = spec.window.unwrap_or(DEFAULT_WINDOW);
        ensure!(
            (1..=MAX_WINDOW).contains(&window),
            "taste pooling window {window} is outside 1..={MAX_WINDOW}"
        );
        let mut weights = [None; 6];
        for (label, weight) in spec.w {
            let kind = EventKind::from_label(&label)
                .with_context(|| format!("taste pooling weighs an unknown event type {label}"))?;
            ensure!(
                weight.is_finite(),
                "taste pooling weight of {label} is not finite"
            );
            if let Some(slot) = weights.get_mut(usize::from(kind.code())) {
                *slot = Some(weight as f32);
            }
        }
        ensure!(
            weights.iter().any(Option::is_some),
            "taste pooling weighs no event type"
        );
        Ok(Self {
            weights,
            tau_days: spec.tau_days,
            window,
            strength: spec.strength,
            min_positives: spec.min_positives.max(1),
        })
    }

    pub(crate) fn serves(&self, positives: Positives) -> bool {
        positives.total >= self.min_positives
    }

    pub(crate) fn user_vector<'a>(
        &self,
        events: &[TasteEvent],
        item: impl Fn(u64) -> Option<&'a [f32]>,
        now_unix: i64,
        dimensions: usize,
    ) -> Option<Vec<f32>> {
        let known: Vec<(&TasteEvent, &[f32])> = events
            .iter()
            .filter_map(|event| {
                let vector = item(event.track).filter(|vector| vector.len() == dimensions)?;
                Some((event, vector))
            })
            .collect();
        let recent = known
            .get(known.len().saturating_sub(self.window)..)
            .unwrap_or(&known);
        let mut sum = vec![0.0f32; dimensions];
        for (event, vector) in recent {
            let Some(weight) = self.weight_of(event.kind) else {
                continue;
            };
            let scale = weight * self.strength_of(event) * self.decay(event.unix_s, now_unix);
            for (total, value) in sum.iter_mut().zip(vector.iter()) {
                *total += scale * value;
            }
        }
        normalized(sum)
    }

    fn strength_of(&self, event: &TasteEvent) -> f32 {
        match self.strength {
            Strength::TypeOnly => 1.0,
            Strength::AbsWeight => event.weight.abs() as f32,
        }
    }

    fn weight_of(&self, kind: EventKind) -> Option<f32> {
        self.weights
            .get(usize::from(kind.code()))
            .copied()
            .flatten()
    }

    fn decay(&self, unix_s: Option<i64>, now_unix: i64) -> f32 {
        let Some(unix_s) = unix_s else {
            return 1.0;
        };
        let age_days = now_unix.saturating_sub(unix_s).max(0) as f64 / SECONDS_PER_DAY;
        (-age_days / self.tau_days).exp() as f32
    }
}

fn normalized(mut vector: Vec<f32>) -> Option<Vec<f32>> {
    let norm = vector.iter().map(|value| value * value).sum::<f32>().sqrt();
    if !norm.is_finite() || norm <= f32::EPSILON {
        return None;
    }
    for value in &mut vector {
        *value /= norm;
    }
    Some(vector)
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    const DAY: i64 = 86_400;

    fn pooling(window: usize) -> Pooling {
        Pooling::from_json(&serde_json::json!({
            "w": {"like": 1.0, "like_import": 0.5, "skip": -0.5, "4": -0.5},
            "tau_days": 30.0,
            "window": window
        }))
        .expect("pooling")
    }

    fn event(track: u64, kind: EventKind, unix_s: Option<i64>) -> TasteEvent {
        TasteEvent {
            track,
            kind,
            unix_s,
            weight: 1.0,
        }
    }

    fn items() -> HashMap<u64, Vec<f32>> {
        HashMap::from([
            (1, vec![1.0, 0.0]),
            (2, vec![0.0, 1.0]),
            (3, vec![-1.0, 0.0]),
        ])
    }

    #[test]
    fn a_liked_track_pulls_the_taste_towards_it_and_a_skip_pushes_it_away() {
        let items = items();
        let events = vec![
            event(1, EventKind::Like, Some(100 * DAY)),
            event(3, EventKind::Skip, Some(100 * DAY)),
        ];

        let vector = pooling(200)
            .user_vector(
                &events,
                |id| items.get(&id).map(Vec::as_slice),
                100 * DAY,
                2,
            )
            .expect("vector");

        assert!((vector[0] - 1.0).abs() < 1e-6);
        assert!(vector[1].abs() < 1e-6);
    }

    #[test]
    fn older_events_weigh_less_and_an_imported_like_does_not_age() {
        let items = items();
        let now = 1_000 * DAY;
        let recent_over_old = vec![
            event(1, EventKind::Like, Some(now - 300 * DAY)),
            event(2, EventKind::Like, Some(now)),
        ];
        let imported = vec![
            event(1, EventKind::LikeImport, None),
            event(2, EventKind::Like, Some(now - 300 * DAY)),
        ];

        let recent = pooling(200)
            .user_vector(
                &recent_over_old,
                |id| items.get(&id).map(Vec::as_slice),
                now,
                2,
            )
            .expect("vector");
        let timeless = pooling(200)
            .user_vector(&imported, |id| items.get(&id).map(Vec::as_slice), now, 2)
            .expect("vector");

        assert!(recent[1] > recent[0]);
        assert!(timeless[0] > timeless[1]);
    }

    #[test]
    fn only_the_last_window_of_events_counts() {
        let items = items();
        let events = vec![
            event(1, EventKind::Like, Some(DAY)),
            event(2, EventKind::Like, Some(DAY)),
        ];

        let vector = pooling(1)
            .user_vector(&events, |id| items.get(&id).map(Vec::as_slice), DAY, 2)
            .expect("vector");

        assert_eq!(vector, vec![0.0, 1.0]);
    }

    fn trained_by_the_worker(max_events: usize) -> Pooling {
        Pooling::from_json(&serde_json::json!({
            "w": {
                "like": 1.0,
                "like_import": 1.0,
                "playlist_add": 1.0,
                "full_play": 1.0,
                "skip": -1.0,
                "dislike": -1.0
            },
            "tau_days": 30.0,
            "max_events": max_events,
            "strength": "abs_weight",
            "decay": "exp_age_over_tau",
            "untimed_decay": 1.0
        }))
        .expect("the pooling block of a worker artifact")
    }

    fn weighted(track: u64, kind: EventKind, weight: f64) -> TasteEvent {
        TasteEvent {
            track,
            kind,
            unix_s: Some(DAY),
            weight,
        }
    }

    #[test]
    fn the_pooling_block_of_a_worker_artifact_keeps_its_window_and_strength() {
        let pooling = trained_by_the_worker(7);

        assert_eq!(pooling.window, 7);
        assert_eq!(pooling.strength, Strength::AbsWeight);
    }

    #[test]
    fn an_event_counts_as_strongly_as_the_api_weighed_it_when_the_worker_trained_so() {
        let items = items();
        let events = vec![
            weighted(1, EventKind::FullPlay, 0.3),
            weighted(2, EventKind::Like, 1.0),
        ];

        let vector = trained_by_the_worker(200)
            .user_vector(&events, |id| items.get(&id).map(Vec::as_slice), DAY, 2)
            .expect("vector");
        let flat = pooling(200)
            .user_vector(&events, |id| items.get(&id).map(Vec::as_slice), DAY, 2)
            .expect("vector");

        assert!((vector[1] / vector[0] - 1.0 / 0.3).abs() < 1e-4);
        assert!(flat[0].abs() < 1e-6);
    }

    #[test]
    fn the_window_is_taken_from_tracks_the_model_knows() {
        let items = items();
        let events = vec![
            weighted(1, EventKind::Like, 1.0),
            weighted(9, EventKind::Like, 1.0),
            weighted(8, EventKind::Like, 1.0),
        ];

        let vector = trained_by_the_worker(1)
            .user_vector(&events, |id| items.get(&id).map(Vec::as_slice), DAY, 2)
            .expect("vector");

        assert_eq!(vector, vec![1.0, 0.0]);
    }

    #[test]
    fn a_short_history_gets_no_vector_when_the_worker_saw_the_model_lose_it() {
        let short = Positives { total: 9, timed: 9 };
        let long = Positives {
            total: 10,
            timed: 2,
        };
        let guarded = Pooling::from_json(&serde_json::json!({
            "w": {"like": 1.0},
            "tau_days": 30.0,
            "min_positives": 10
        }))
        .expect("pooling");

        assert!(!guarded.serves(short));
        assert!(guarded.serves(long));
        assert!(trained_by_the_worker(200).serves(Positives { total: 1, timed: 0 }));
        assert!(!trained_by_the_worker(200).serves(Positives::default()));
    }

    #[test]
    fn a_strength_the_worker_never_writes_is_refused() {
        let spec = serde_json::json!({"w": {"like": 1.0}, "tau_days": 30.0, "strength": "square"});

        assert!(Pooling::from_json(&spec).is_err());
    }

    #[test]
    fn a_history_without_known_tracks_has_no_taste() {
        let items = items();
        let events = vec![
            event(9, EventKind::Like, Some(DAY)),
            event(1, EventKind::Dislike, Some(DAY)),
        ];

        assert!(
            pooling(200)
                .user_vector(&events, |id| items.get(&id).map(Vec::as_slice), DAY, 2)
                .is_none()
        );
    }

    #[test]
    fn a_pooling_the_worker_could_not_have_meant_is_refused() {
        for spec in [
            serde_json::json!({"w": {"like": 1.0}, "tau_days": 0.0}),
            serde_json::json!({"w": {"liked": 1.0}, "tau_days": 30.0}),
            serde_json::json!({"w": {}, "tau_days": 30.0}),
            serde_json::json!({"w": {"like": 1.0}, "tau_days": 30.0, "window": 0}),
            serde_json::json!({"tau_days": 30.0}),
        ] {
            assert!(Pooling::from_json(&spec).is_err(), "{spec} was accepted");
        }
    }
}
