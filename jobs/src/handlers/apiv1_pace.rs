use std::sync::{Mutex, OnceLock, PoisonError};
use std::time::{Duration, Instant};

const MIN_RATE: f64 = 0.2;
const LIMIT_PAUSE: Duration = Duration::from_secs(20);
const CALM_BEFORE_RAISE: Duration = Duration::from_secs(60);
const RAISE_EVERY: Duration = Duration::from_secs(30);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Use {
    Read,
    Write,
}

struct Pace {
    rate: f64,
    ceiling: f64,
    next: Instant,
    paused_until: Instant,
    limited_at: Option<Instant>,
    raised_at: Instant,
}

impl Pace {
    fn new(now: Instant, start: f64, ceiling: f64) -> Self {
        Self {
            rate: start,
            ceiling,
            next: now,
            paused_until: now,
            limited_at: None,
            raised_at: now,
        }
    }

    fn reserve(&mut self, now: Instant) -> Duration {
        let slot = self.next.max(now).max(self.paused_until);
        self.next = slot + Duration::from_secs_f64(1.0 / self.rate);
        slot.saturating_duration_since(now)
    }

    fn limited(&mut self, now: Instant) {
        if self
            .limited_at
            .is_some_and(|at| now.saturating_duration_since(at) < LIMIT_PAUSE)
        {
            return;
        }
        self.rate = (self.rate / 2.0).max(MIN_RATE);
        self.paused_until = now + LIMIT_PAUSE;
        self.limited_at = Some(now);
    }

    fn succeeded(&mut self, now: Instant) {
        let calm = self
            .limited_at
            .is_none_or(|at| now.saturating_duration_since(at) >= CALM_BEFORE_RAISE);
        if calm && now.saturating_duration_since(self.raised_at) >= RAISE_EVERY {
            self.rate = (self.rate * 1.15).min(self.ceiling);
            self.raised_at = now;
        }
    }
}

fn pace(usage: Use) -> &'static Mutex<Pace> {
    static READS: OnceLock<Mutex<Pace>> = OnceLock::new();
    static WRITES: OnceLock<Mutex<Pace>> = OnceLock::new();
    match usage {
        Use::Read => READS.get_or_init(|| Mutex::new(Pace::new(Instant::now(), 2.0, 8.0))),
        Use::Write => WRITES.get_or_init(|| Mutex::new(Pace::new(Instant::now(), 1.0, 4.0))),
    }
}

pub async fn wait_for_turn(usage: Use) {
    if cfg!(test) {
        return;
    }
    let wait = pace(usage)
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .reserve(Instant::now());
    if !wait.is_zero() {
        tokio::time::sleep(wait).await;
    }
}

pub fn record(usage: Use, rate_limited: bool) {
    if cfg!(test) {
        return;
    }
    let now = Instant::now();
    if !rate_limited {
        pace(usage)
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .succeeded(now);
        return;
    }
    for shared in [Use::Read, Use::Write] {
        let mut pace = pace(shared).lock().unwrap_or_else(PoisonError::into_inner);
        pace.limited(now);
        tracing::warn!(hit_by = ?usage, slowed = ?shared, rate = pace.rate, "soundcloud api rate limit hit, slowing down");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const START: f64 = 2.0;
    const CEILING: f64 = 6.0;

    #[test]
    fn requests_are_spaced_by_the_current_rate() {
        let now = Instant::now();
        let mut pace = Pace::new(now, START, CEILING);

        assert_eq!(pace.reserve(now), Duration::ZERO);
        let second = pace.reserve(now);

        assert!((second.as_secs_f64() - 1.0 / START).abs() < 0.001);
    }

    #[test]
    fn a_rate_limit_halves_the_rate_once_and_holds_everything_back() {
        let now = Instant::now();
        let mut pace = Pace::new(now, START, CEILING);

        pace.limited(now);
        pace.limited(now + Duration::from_secs(1));

        assert!((pace.rate - START / 2.0).abs() < f64::EPSILON);
        assert!(pace.reserve(now + Duration::from_secs(1)) >= Duration::from_secs(19));
    }

    #[test]
    fn the_rate_climbs_back_only_after_a_calm_minute() {
        let now = Instant::now();
        let mut pace = Pace::new(now, START, CEILING);
        pace.limited(now);
        let slowed = pace.rate;

        pace.succeeded(now + Duration::from_secs(40));
        assert!((pace.rate - slowed).abs() < f64::EPSILON);

        pace.succeeded(now + Duration::from_secs(61));
        assert!(pace.rate > slowed);
    }
}
