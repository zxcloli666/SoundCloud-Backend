use std::sync::{Mutex, OnceLock, PoisonError};
use std::time::{Duration, Instant};

const START_RATE: f64 = 4.0;
const MIN_RATE: f64 = 0.5;
const MAX_RATE: f64 = 12.0;
const WRITE_SHARE: f64 = 0.6;
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
    next_read: Instant,
    next_write: Instant,
    paused_until: Instant,
    limited_at: Option<Instant>,
    raised_at: Instant,
}

impl Pace {
    fn new(now: Instant) -> Self {
        Self {
            rate: START_RATE,
            next_read: now,
            next_write: now,
            paused_until: now,
            limited_at: None,
            raised_at: now,
        }
    }

    fn reserve(&mut self, usage: Use, now: Instant) -> Duration {
        let share = match usage {
            Use::Read => 1.0 - WRITE_SHARE,
            Use::Write => WRITE_SHARE,
        };
        let interval = Duration::from_secs_f64(1.0 / (self.rate * share));
        let earliest = now.max(self.paused_until);
        let next = match usage {
            Use::Read => &mut self.next_read,
            Use::Write => &mut self.next_write,
        };
        let slot = (*next).max(earliest);
        *next = slot + interval;
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
            self.rate = (self.rate * 1.15).min(MAX_RATE);
            self.raised_at = now;
        }
    }
}

fn pace() -> &'static Mutex<Pace> {
    static PACE: OnceLock<Mutex<Pace>> = OnceLock::new();
    PACE.get_or_init(|| Mutex::new(Pace::new(Instant::now())))
}

pub async fn wait_for_turn(usage: Use) {
    if cfg!(test) {
        return;
    }
    let wait = pace()
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .reserve(usage, Instant::now());
    if !wait.is_zero() {
        tokio::time::sleep(wait).await;
    }
}

pub fn record(rate_limited: bool) {
    if cfg!(test) {
        return;
    }
    let mut pace = pace().lock().unwrap_or_else(PoisonError::into_inner);
    let now = Instant::now();
    if rate_limited {
        pace.limited(now);
        tracing::warn!(
            rate = pace.rate,
            "soundcloud api rate limit hit, slowing down"
        );
    } else {
        pace.succeeded(now);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_and_reads_are_spaced_by_their_own_share_of_the_rate() {
        let now = Instant::now();
        let mut pace = Pace::new(now);

        assert_eq!(pace.reserve(Use::Write, now), Duration::ZERO);
        assert_eq!(pace.reserve(Use::Read, now), Duration::ZERO);
        let second_write = pace.reserve(Use::Write, now);
        let second_read = pace.reserve(Use::Read, now);

        assert!(second_write < second_read);
        assert!((second_write.as_secs_f64() - 1.0 / (START_RATE * WRITE_SHARE)).abs() < 0.001);
    }

    #[test]
    fn a_rate_limit_halves_the_rate_once_and_holds_everything_back() {
        let now = Instant::now();
        let mut pace = Pace::new(now);

        pace.limited(now);
        pace.limited(now + Duration::from_secs(1));

        assert!((pace.rate - START_RATE / 2.0).abs() < f64::EPSILON);
        assert!(pace.reserve(Use::Write, now + Duration::from_secs(1)) >= Duration::from_secs(19));
    }

    #[test]
    fn the_rate_climbs_back_only_after_a_calm_minute() {
        let now = Instant::now();
        let mut pace = Pace::new(now);
        pace.limited(now);
        let slowed = pace.rate;

        pace.succeeded(now + Duration::from_secs(40));
        assert!((pace.rate - slowed).abs() < f64::EPSILON);

        pace.succeeded(now + Duration::from_secs(61));
        assert!(pace.rate > slowed);
    }
}
