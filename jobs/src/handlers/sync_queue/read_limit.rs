use std::collections::HashMap;
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use uuid::Uuid;

const FIRST_GAP: Duration = Duration::from_secs(10);
const LONGEST_GAP: Duration = Duration::from_secs(5 * 60);
const FREE_BELOW: Duration = Duration::from_secs(1);
const SHORTEN: f64 = 0.8;

struct Limit {
    gap: Duration,
    next: Instant,
}

#[derive(Default)]
pub struct AppReadLimits {
    apps: Mutex<HashMap<Uuid, Limit>>,
}

impl AppReadLimits {
    pub fn limited(&self, app: Uuid, seconds: i64) -> i64 {
        let pause = Duration::from_secs(u64::try_from(seconds).unwrap_or(0).max(1));
        whole_seconds(self.limited_at(app, pause, Instant::now()))
    }

    pub fn admit(&self, app: Uuid) -> Option<i64> {
        self.admit_at(app, Instant::now()).map(whole_seconds)
    }

    pub fn answered(&self, app: Uuid) {
        let mut apps = self.apps.lock().unwrap_or_else(PoisonError::into_inner);
        let Some(limit) = apps.get_mut(&app) else {
            return;
        };
        limit.gap = limit.gap.mul_f64(SHORTEN);
        if limit.gap < FREE_BELOW {
            apps.remove(&app);
        }
    }

    fn limited_at(&self, app: Uuid, pause: Duration, now: Instant) -> Duration {
        let mut apps = self.apps.lock().unwrap_or_else(PoisonError::into_inner);
        let limit = apps.entry(app).or_insert(Limit {
            gap: FIRST_GAP / 2,
            next: now,
        });
        limit.gap = (limit.gap * 2).clamp(FIRST_GAP, LONGEST_GAP);
        limit.next = limit.next.max(now + pause.max(limit.gap));
        limit.next.saturating_duration_since(now)
    }

    fn admit_at(&self, app: Uuid, now: Instant) -> Option<Duration> {
        let mut apps = self.apps.lock().unwrap_or_else(PoisonError::into_inner);
        let limit = apps.get_mut(&app)?;
        if limit.next > now {
            return Some(limit.next.saturating_duration_since(now));
        }
        limit.next = now + limit.gap;
        None
    }
}

fn whole_seconds(wait: Duration) -> i64 {
    let seconds = wait.as_secs() + u64::from(wait.subsec_nanos() > 0);
    i64::try_from(seconds).unwrap_or(i64::MAX).max(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAUSE: Duration = Duration::from_secs(300);

    fn gap(limits: &AppReadLimits, app: Uuid) -> Option<Duration> {
        limits.apps.lock().unwrap().get(&app).map(|limit| limit.gap)
    }

    #[test]
    fn an_app_that_was_never_limited_reads_freely() {
        let limits = AppReadLimits::default();
        let app = Uuid::now_v7();

        limits.answered(app);

        assert_eq!(limits.admit_at(app, Instant::now()), None);
        assert_eq!(limits.admit_at(app, Instant::now()), None);
    }

    #[test]
    fn a_limited_app_waits_out_the_pause_and_then_gets_one_read_per_gap() {
        let limits = AppReadLimits::default();
        let app = Uuid::now_v7();
        let now = Instant::now();

        assert_eq!(limits.limited_at(app, PAUSE, now), PAUSE);

        assert_eq!(
            limits.admit_at(app, now + PAUSE - Duration::from_secs(1)),
            Some(Duration::from_secs(1))
        );
        assert_eq!(limits.admit_at(app, now + PAUSE), None);
        assert_eq!(limits.admit_at(app, now + PAUSE), Some(FIRST_GAP));
        assert_eq!(limits.admit_at(app, now + PAUSE + FIRST_GAP), None);
        assert_eq!(limits.admit_at(Uuid::now_v7(), now), None);
    }

    #[test]
    fn answers_shorten_the_gap_until_the_app_reads_freely_again() {
        let limits = AppReadLimits::default();
        let app = Uuid::now_v7();
        let now = Instant::now();
        limits.limited_at(app, PAUSE, now);

        limits.answered(app);
        assert_eq!(gap(&limits, app), Some(FIRST_GAP.mul_f64(SHORTEN)));
        for _ in 0..10 {
            limits.answered(app);
        }

        assert_eq!(gap(&limits, app), None);
        assert_eq!(limits.admit_at(app, now), None);
    }

    #[test]
    fn every_new_limit_doubles_the_gap_up_to_five_minutes() {
        let limits = AppReadLimits::default();
        let app = Uuid::now_v7();
        let now = Instant::now();

        limits.limited_at(app, PAUSE, now);
        limits.limited_at(app, PAUSE, now);
        assert_eq!(gap(&limits, app), Some(FIRST_GAP * 2));
        for _ in 0..10 {
            limits.limited_at(app, PAUSE, now);
        }
        assert_eq!(gap(&limits, app), Some(LONGEST_GAP));

        assert_eq!(limits.admit_at(app, now + PAUSE), None);
        assert_eq!(limits.admit_at(app, now + PAUSE), Some(LONGEST_GAP));
    }

    #[test]
    fn a_short_pause_never_cuts_a_longer_one() {
        let limits = AppReadLimits::default();
        let app = Uuid::now_v7();
        let now = Instant::now();

        limits.limited_at(app, PAUSE, now);

        assert_eq!(limits.limited_at(app, Duration::from_secs(30), now), PAUSE);
    }

    #[test]
    fn waits_round_up_to_whole_seconds() {
        assert_eq!(whole_seconds(Duration::from_millis(1)), 1);
        assert_eq!(whole_seconds(Duration::from_millis(1500)), 2);
        assert_eq!(whole_seconds(Duration::from_secs(300)), 300);
    }
}
