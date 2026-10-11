use std::collections::HashMap;
use std::hash::Hash;
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

const MAX_DOUBLINGS: u32 = 4;

struct Pause {
    until: Instant,
    strikes: u32,
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

impl<K: Eq + Hash> WritePauses<K> {
    pub fn pause(&self, key: K, seconds: i64) {
        self.hold(key, seconds, false);
    }

    pub fn pause_longer_each_time(&self, key: K, seconds: i64) -> i64 {
        self.hold(key, seconds, true)
    }

    pub fn clear(&self, key: &K) {
        self.paused
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(key);
    }

    pub fn remaining_seconds(&self, key: &K) -> Option<i64> {
        let mut paused = self.paused.lock().unwrap_or_else(PoisonError::into_inner);
        let pause = paused.get(key)?;
        let remaining = pause.until.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            if pause.strikes == 0 {
                paused.remove(key);
            }
            return None;
        }
        Some(
            i64::try_from(remaining.as_secs())
                .unwrap_or(i64::MAX)
                .max(1),
        )
    }

    fn hold(&self, key: K, seconds: i64, escalate: bool) -> i64 {
        let now = Instant::now();
        let mut paused = self.paused.lock().unwrap_or_else(PoisonError::into_inner);
        let pause = paused.entry(key).or_insert(Pause {
            until: now,
            strikes: 0,
        });
        let seconds = u64::try_from(seconds).unwrap_or(0).max(1);
        let seconds = if escalate && pause.until <= now {
            let longer = seconds << pause.strikes.min(MAX_DOUBLINGS);
            pause.strikes = pause.strikes.saturating_add(1);
            longer
        } else {
            seconds
        };
        pause.until = pause.until.max(now + Duration::from_secs(seconds));
        i64::try_from(seconds).unwrap_or(i64::MAX)
    }
}

#[cfg(test)]
mod tests {
    use uuid::Uuid;

    use super::*;

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
    fn a_refusal_after_each_expired_pause_doubles_the_next_one_until_a_success_clears_it() {
        let pauses = WritePauses::default();

        assert_eq!(pauses.pause_longer_each_time("likes", 60), 60);
        assert_eq!(pauses.pause_longer_each_time("likes", 60), 60);
        pauses
            .paused
            .lock()
            .unwrap()
            .get_mut("likes")
            .unwrap()
            .until = Instant::now();
        assert_eq!(pauses.remaining_seconds(&"likes"), None);
        assert_eq!(pauses.pause_longer_each_time("likes", 60), 120);

        pauses.clear(&"likes");
        assert_eq!(pauses.pause_longer_each_time("likes", 60), 60);
    }
}
