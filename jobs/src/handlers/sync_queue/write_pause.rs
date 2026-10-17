use std::collections::HashMap;
use std::hash::Hash;
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

pub const MAX_PAUSE: Duration = Duration::from_secs(2 * 60 * 60);
const PROBE_HOLD: Duration = Duration::from_secs(120);

struct Pause {
    until: Instant,
    strikes: u32,
    probing: bool,
}

#[derive(Debug, Eq, PartialEq)]
pub enum Admission {
    Open,
    Probe,
    Paused(i64),
}

pub struct WritePauses<K> {
    paused: Mutex<HashMap<K, Pause>>,
}

impl<K> Default for WritePauses<K> {
    fn default() -> Self {
        Self {
            paused: Mutex::new(HashMap::new()),
        }
    }
}

impl<K: Clone + Eq + Hash> WritePauses<K> {
    pub fn pause(&self, key: K, seconds: i64) {
        self.hold(key, seconds, false);
    }

    pub fn pause_longer_each_time(&self, key: K, seconds: i64) -> (i64, u32) {
        let seconds = self.hold(key.clone(), seconds, true);
        (seconds, self.strikes(&key))
    }

    pub fn pause_refused(&self, key: K, seconds: i64) -> (i64, u32) {
        let now = Instant::now();
        let mut paused = self.paused.lock().unwrap_or_else(PoisonError::into_inner);
        let pause = paused.entry(key).or_insert(Pause {
            until: now,
            strikes: 0,
            probing: false,
        });
        let hold = Duration::from_secs(u64::try_from(seconds).unwrap_or(0).max(1)).min(MAX_PAUSE);
        pause.until = pause.until.max(now + hold);
        pause.strikes = pause.strikes.max(1);
        pause.probing = false;
        (
            whole_seconds(pause.until.saturating_duration_since(now)),
            pause.strikes,
        )
    }

    pub fn restore(&self, key: K, remaining: Duration, strikes: u32) {
        self.paused
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(
                key,
                Pause {
                    until: Instant::now() + remaining.min(MAX_PAUSE),
                    strikes: strikes.max(1),
                    probing: false,
                },
            );
    }

    fn strikes(&self, key: &K) -> u32 {
        self.paused
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(key)
            .map_or(0, |pause| pause.strikes)
    }

    pub fn admit(&self, key: &K) -> Admission {
        let now = Instant::now();
        let mut paused = self.paused.lock().unwrap_or_else(PoisonError::into_inner);
        let Some(pause) = paused.get_mut(key) else {
            return Admission::Open;
        };
        let remaining = pause.until.saturating_duration_since(now);
        if !remaining.is_zero() {
            return Admission::Paused(whole_seconds(remaining));
        }
        if pause.strikes == 0 {
            paused.remove(key);
            return Admission::Open;
        }
        pause.until = now + PROBE_HOLD;
        pause.probing = true;
        Admission::Probe
    }

    pub fn succeeded(&self, key: &K, spacing: Option<Duration>) -> bool {
        let mut paused = self.paused.lock().unwrap_or_else(PoisonError::into_inner);
        let was_refused = paused.get(key).is_some_and(|pause| pause.strikes > 0);
        match spacing {
            Some(spacing) => {
                paused.insert(
                    key.clone(),
                    Pause {
                        until: Instant::now() + spacing,
                        strikes: 0,
                        probing: false,
                    },
                );
            }
            None => {
                paused.remove(key);
            }
        }
        was_refused
    }

    pub fn remaining_seconds(&self, key: &K) -> Option<i64> {
        match self.admit(key) {
            Admission::Paused(seconds) => Some(seconds),
            Admission::Open | Admission::Probe => None,
        }
    }

    pub fn held(&self) -> Vec<K> {
        let now = Instant::now();
        self.paused
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .filter(|(_, pause)| pause.until > now)
            .map(|(key, _)| key.clone())
            .collect()
    }

    pub fn next_spacing_end(&self) -> Option<Duration> {
        let now = Instant::now();
        self.paused
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .values()
            .filter(|pause| pause.strikes == 0 && pause.until > now)
            .map(|pause| pause.until.saturating_duration_since(now))
            .min()
    }

    fn hold(&self, key: K, seconds: i64, escalate: bool) -> i64 {
        let now = Instant::now();
        let mut paused = self.paused.lock().unwrap_or_else(PoisonError::into_inner);
        let pause = paused.entry(key).or_insert(Pause {
            until: now,
            strikes: 0,
            probing: false,
        });
        let base = Duration::from_secs(u64::try_from(seconds).unwrap_or(0).max(1));
        if escalate && (pause.probing || pause.strikes == 0 || pause.until <= now) {
            let longer = base
                .checked_mul(1_u32.checked_shl(pause.strikes).unwrap_or(u32::MAX))
                .unwrap_or(MAX_PAUSE)
                .min(MAX_PAUSE);
            pause.strikes = pause.strikes.saturating_add(1);
            pause.probing = false;
            pause.until = now + longer;
            return whole_seconds(longer);
        }
        pause.until = pause.until.max(now + base);
        whole_seconds(base)
    }
}

fn whole_seconds(duration: Duration) -> i64 {
    i64::try_from(duration.as_secs()).unwrap_or(i64::MAX).max(1)
}

#[cfg(test)]
mod tests {
    use uuid::Uuid;

    use super::*;

    fn expire<K: Clone + Eq + Hash>(pauses: &WritePauses<K>, key: &K) {
        pauses.paused.lock().unwrap().get_mut(key).unwrap().until = Instant::now();
    }

    #[test]
    fn a_paused_app_reports_the_wait_and_other_apps_stay_open() {
        let pauses = WritePauses::default();
        let paused = Uuid::now_v7();

        pauses.pause(paused, 300);

        assert!(
            pauses
                .remaining_seconds(&paused)
                .is_some_and(|seconds| seconds > 290)
        );
        assert_eq!(pauses.remaining_seconds(&Uuid::now_v7()), None);
    }

    #[test]
    fn a_shorter_pause_never_shortens_a_longer_one() {
        let pauses = WritePauses::default();
        let app = Uuid::now_v7();

        pauses.pause(app, 600);
        pauses.pause(app, 5);

        assert!(
            pauses
                .remaining_seconds(&app)
                .is_some_and(|seconds| seconds > 590)
        );
    }

    #[test]
    fn after_a_refusal_only_one_probe_goes_out_and_its_failure_doubles_the_wait() {
        let pauses = WritePauses::default();

        assert_eq!(pauses.pause_longer_each_time("likes", 60).0, 60);
        assert_eq!(pauses.pause_longer_each_time("likes", 60).0, 60);
        expire(&pauses, &"likes");

        assert_eq!(pauses.admit(&"likes"), Admission::Probe);
        assert!(matches!(pauses.admit(&"likes"), Admission::Paused(_)));
        assert_eq!(pauses.pause_longer_each_time("likes", 60), (120, 2));
        expire(&pauses, &"likes");
        assert_eq!(pauses.admit(&"likes"), Admission::Probe);
        assert_eq!(pauses.pause_longer_each_time("likes", 60), (240, 3));
    }

    #[test]
    fn a_restored_refusal_keeps_its_wait_and_its_place_in_the_backoff() {
        let pauses = WritePauses::default();

        pauses.restore("likes", Duration::from_secs(600), 4);

        assert!(matches!(pauses.admit(&"likes"), Admission::Paused(seconds) if seconds > 590));
        expire(&pauses, &"likes");
        assert_eq!(pauses.admit(&"likes"), Admission::Probe);
        assert_eq!(pauses.pause_longer_each_time("likes", 60), (960, 5));
        assert!(pauses.succeeded(&"likes", None));
        assert!(!pauses.succeeded(&"likes", None));
    }

    #[test]
    fn the_wait_stops_growing_at_two_hours() {
        let pauses = WritePauses::default();
        let mut longest = 0;

        for _ in 0..20 {
            longest = pauses.pause_longer_each_time("likes", 60).0;
            expire(&pauses, &"likes");
        }

        assert_eq!(longest, 2 * 60 * 60);
    }

    #[test]
    fn a_refused_pause_lets_one_probe_through_and_is_cleared_by_success() {
        let pauses = WritePauses::default();

        assert_eq!(pauses.pause_refused("likes", 6 * 60 * 60), (2 * 60 * 60, 1));
        assert!(matches!(
            pauses.pause_refused("likes", 60),
            (seconds, 1) if seconds > 2 * 60 * 60 - 10
        ));
        expire(&pauses, &"likes");

        assert_eq!(pauses.admit(&"likes"), Admission::Probe);
        assert!(matches!(pauses.admit(&"likes"), Admission::Paused(_)));
        assert!(pauses.succeeded(&"likes", None));
        assert_eq!(pauses.admit(&"likes"), Admission::Open);
    }

    #[test]
    fn a_restored_pause_never_exceeds_two_hours() {
        let pauses = WritePauses::default();

        pauses.restore("likes", Duration::from_secs(20 * 60 * 60), 6);

        assert!(matches!(
            pauses.admit(&"likes"),
            Admission::Paused(seconds) if seconds <= 2 * 60 * 60
        ));
        expire(&pauses, &"likes");
        assert_eq!(pauses.admit(&"likes"), Admission::Probe);
    }

    #[test]
    fn a_successful_probe_opens_the_action_again_at_its_spacing() {
        let pauses = WritePauses::default();
        pauses.pause_longer_each_time("likes", 60);
        expire(&pauses, &"likes");
        assert_eq!(pauses.admit(&"likes"), Admission::Probe);

        pauses.succeeded(&"likes", Some(Duration::from_secs(5)));

        assert!(matches!(pauses.admit(&"likes"), Admission::Paused(seconds) if seconds <= 5));
        assert_eq!(pauses.held(), vec!["likes"]);
        assert!(pauses.next_spacing_end().is_some());
        expire(&pauses, &"likes");
        assert_eq!(pauses.admit(&"likes"), Admission::Open);
        assert_eq!(pauses.pause_longer_each_time("likes", 60).0, 60);
    }

    #[test]
    fn a_refusal_during_the_spacing_hold_still_starts_the_backoff() {
        let pauses = WritePauses::default();
        pauses.pause("likes", 5);

        assert_eq!(pauses.pause_longer_each_time("likes", 60).0, 60);
        expire(&pauses, &"likes");

        assert_eq!(pauses.admit(&"likes"), Admission::Probe);
        assert_eq!(pauses.next_spacing_end(), None);
    }
}
