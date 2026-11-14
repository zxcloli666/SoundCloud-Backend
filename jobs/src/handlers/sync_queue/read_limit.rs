use std::collections::HashMap;
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use uuid::Uuid;

use super::read_line::{Line, Waiter};

const FIRST_GAP: Duration = Duration::from_secs(10);
const LONGEST_GAP: Duration = Duration::from_secs(5 * 60);
const FREE_BELOW: Duration = Duration::from_millis(125);
const SHORTEN: f64 = 0.8;
const SHORTEN_EVERY: Duration = Duration::from_secs(30);

struct Limit {
    gap: Duration,
    next: Instant,
    changed_at: Instant,
}

#[derive(Default)]
struct Apps {
    limits: HashMap<Uuid, Limit>,
    lines: HashMap<Uuid, Line>,
}

#[derive(Default)]
pub struct AppReadLimits {
    apps: Mutex<Apps>,
}

impl AppReadLimits {
    pub fn limited(&self, app: Uuid, seconds: i64) -> i64 {
        let pause = Duration::from_secs(u64::try_from(seconds).unwrap_or(0).max(1));
        whole_seconds(self.limited_at(app, pause, Instant::now()))
    }

    pub fn admit(&self, app: Uuid) -> Option<i64> {
        self.admit_at(app, Instant::now()).map(whole_seconds)
    }

    pub fn turn(&self, app: Uuid, waiter: Waiter, cooldown_seconds: Option<i64>) -> Option<i64> {
        let cooldown =
            Duration::from_secs(u64::try_from(cooldown_seconds.unwrap_or(0)).unwrap_or(0));
        self.turn_at(app, waiter, cooldown, Instant::now())
            .map(whole_seconds)
    }

    pub fn answered(&self, app: Uuid) {
        self.answered_at(app, Instant::now());
    }

    fn answered_at(&self, app: Uuid, now: Instant) {
        let mut apps = self.apps.lock().unwrap_or_else(PoisonError::into_inner);
        let Some(limit) = apps.limits.get_mut(&app) else {
            return;
        };
        if now.saturating_duration_since(limit.changed_at) < SHORTEN_EVERY {
            return;
        }
        limit.gap = limit.gap.mul_f64(SHORTEN);
        limit.changed_at = now;
        if limit.gap < FREE_BELOW {
            apps.limits.remove(&app);
        }
    }

    fn limited_at(&self, app: Uuid, pause: Duration, now: Instant) -> Duration {
        let mut apps = self.apps.lock().unwrap_or_else(PoisonError::into_inner);
        let limit = apps.limits.entry(app).or_insert(Limit {
            gap: FIRST_GAP / 2,
            next: now,
            changed_at: now,
        });
        limit.gap = (limit.gap * 2).clamp(FIRST_GAP, LONGEST_GAP);
        limit.next = limit.next.max(now + pause.max(limit.gap));
        limit.changed_at = limit.next;
        limit.next.saturating_duration_since(now)
    }

    fn admit_at(&self, app: Uuid, now: Instant) -> Option<Duration> {
        let mut apps = self.apps.lock().unwrap_or_else(PoisonError::into_inner);
        let limit = apps.limits.get_mut(&app)?;
        if limit.next > now {
            return Some(limit.next.saturating_duration_since(now));
        }
        limit.next = now + limit.gap;
        None
    }

    fn turn_at(
        &self,
        app: Uuid,
        waiter: Waiter,
        cooldown: Duration,
        now: Instant,
    ) -> Option<Duration> {
        let mut apps = self.apps.lock().unwrap_or_else(PoisonError::into_inner);
        let apps = &mut *apps;
        let (opens_in, gap) = match apps.limits.get(&app) {
            Some(limit) => (
                limit.next.saturating_duration_since(now).max(cooldown),
                limit.gap,
            ),
            None if cooldown.is_zero() => {
                apps.lines.remove(&app);
                return None;
            }
            None => (cooldown, FIRST_GAP),
        };
        let wait = apps
            .lines
            .entry(app)
            .or_default()
            .turn(waiter, opens_in, gap, now);
        if wait.is_none()
            && let Some(limit) = apps.limits.get_mut(&app)
        {
            limit.next = now + limit.gap;
        }
        wait
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
        limits
            .apps
            .lock()
            .unwrap()
            .limits
            .get(&app)
            .map(|limit| limit.gap)
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
    fn answers_shorten_the_gap_once_per_half_minute_until_the_app_reads_freely_again() {
        let limits = AppReadLimits::default();
        let app = Uuid::now_v7();
        let now = Instant::now();
        limits.limited_at(app, PAUSE, now);
        let resumed = now + PAUSE;

        for answer in 0..20 {
            limits.answered_at(app, resumed + Duration::from_secs(answer));
        }
        assert_eq!(gap(&limits, app), Some(FIRST_GAP));

        limits.answered_at(app, resumed + SHORTEN_EVERY);
        limits.answered_at(app, resumed + SHORTEN_EVERY);
        assert_eq!(gap(&limits, app), Some(FIRST_GAP.mul_f64(SHORTEN)));

        for step in 2..20 {
            limits.answered_at(app, resumed + SHORTEN_EVERY * step);
        }
        assert!(gap(&limits, app).is_some_and(|gap| gap < Duration::from_millis(200)));
        limits.answered_at(app, resumed + SHORTEN_EVERY * 20);

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

    fn waiter(priority: i16) -> Waiter {
        Waiter {
            job: Uuid::now_v7(),
            priority,
        }
    }

    #[test]
    fn an_app_that_reads_freely_keeps_no_line() {
        let limits = AppReadLimits::default();
        let app = Uuid::now_v7();
        let now = Instant::now();

        assert_eq!(limits.turn_at(app, waiter(20), Duration::ZERO, now), None);
        assert!(limits.apps.lock().unwrap().lines.is_empty());
    }

    #[test]
    fn a_limited_app_gives_each_waiter_its_own_turn_and_one_read_per_gap() {
        let limits = AppReadLimits::default();
        let app = Uuid::now_v7();
        let now = Instant::now();
        limits.limited_at(app, PAUSE, now);
        let first = waiter(20);
        let second = waiter(20);

        assert_eq!(limits.turn_at(app, first, Duration::ZERO, now), Some(PAUSE));
        assert_eq!(
            limits.turn_at(app, second, Duration::ZERO, now + Duration::from_millis(1)),
            Some(PAUSE - Duration::from_millis(1) + FIRST_GAP)
        );

        let opened = now + PAUSE;
        assert_eq!(limits.turn_at(app, first, Duration::ZERO, opened), None);
        assert_eq!(
            limits.turn_at(app, second, Duration::ZERO, opened),
            Some(FIRST_GAP)
        );
        assert_eq!(
            limits.turn_at(app, second, Duration::ZERO, opened + FIRST_GAP),
            None
        );
    }

    #[test]
    fn a_database_cooldown_lines_waiters_up_even_without_a_read_limit() {
        let limits = AppReadLimits::default();
        let app = Uuid::now_v7();
        let now = Instant::now();
        let cooldown = Duration::from_secs(1800);

        assert_eq!(
            limits.turn_at(app, waiter(20), cooldown, now),
            Some(cooldown)
        );
        assert_eq!(
            limits.turn_at(app, waiter(20), cooldown, now + Duration::from_millis(1)),
            Some(cooldown + FIRST_GAP)
        );
    }

    #[test]
    fn a_longer_database_cooldown_overrides_the_read_pause() {
        let limits = AppReadLimits::default();
        let app = Uuid::now_v7();
        let now = Instant::now();
        limits.limited_at(app, PAUSE, now);

        assert_eq!(
            limits.turn_at(app, waiter(20), PAUSE * 3, now),
            Some(PAUSE * 3)
        );
    }

    #[test]
    fn waits_round_up_to_whole_seconds() {
        assert_eq!(whole_seconds(Duration::from_millis(1)), 1);
        assert_eq!(whole_seconds(Duration::from_millis(1500)), 2);
        assert_eq!(whole_seconds(Duration::from_secs(300)), 300);
    }
}
