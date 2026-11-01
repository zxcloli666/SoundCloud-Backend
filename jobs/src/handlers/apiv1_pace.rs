use std::sync::{Mutex, OnceLock, PoisonError};
use std::time::{Duration, Instant};

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
    raised_at: Instant,
}

impl Pace {
    fn new(now: Instant, start: f64, ceiling: f64) -> Self {
        Self {
            rate: start,
            ceiling,
            next: now,
            raised_at: now,
        }
    }

    fn reserve(&mut self, now: Instant) -> Duration {
        let slot = self.next.max(now);
        self.next = slot + Duration::from_secs_f64(1.0 / self.rate);
        slot.saturating_duration_since(now)
    }

    fn reserve_within(&mut self, now: Instant, patience: Duration) -> Result<Duration, Duration> {
        let wait = self.next.saturating_duration_since(now);
        if wait > patience {
            return Err(wait);
        }
        Ok(self.reserve(now))
    }

    fn answered(&mut self, now: Instant) {
        if now.saturating_duration_since(self.raised_at) >= RAISE_EVERY {
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

pub async fn turn_within(usage: Use, patience: Duration) -> Result<(), Duration> {
    if cfg!(test) {
        return Ok(());
    }
    let wait = pace(usage)
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .reserve_within(Instant::now(), patience)?;
    if !wait.is_zero() {
        tokio::time::sleep(wait).await;
    }
    Ok(())
}

pub fn answered(usage: Use) {
    if cfg!(test) {
        return;
    }
    pace(usage)
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .answered(Instant::now());
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
    fn the_rate_climbs_to_the_ceiling_and_stays_there() {
        let now = Instant::now();
        let mut pace = Pace::new(now, START, CEILING);

        pace.answered(now + RAISE_EVERY / 2);
        assert!((pace.rate - START).abs() < f64::EPSILON);

        for step in 1..=40 {
            pace.answered(now + RAISE_EVERY * step);
        }
        assert!((pace.rate - CEILING).abs() < f64::EPSILON);
    }

    #[test]
    fn a_caller_that_would_wait_past_its_patience_takes_no_turn() {
        let now = Instant::now();
        let mut pace = Pace::new(now, START, CEILING);
        for _ in 0..3 {
            pace.reserve(now);
        }

        let refused = pace.reserve_within(now, Duration::from_secs(1));
        let waited = pace.reserve_within(now, Duration::from_secs(2));

        assert_eq!(refused, Err(Duration::from_millis(1500)));
        assert_eq!(waited, Ok(Duration::from_millis(1500)));
    }

    #[test]
    fn a_crowd_never_queues_past_the_patience() {
        let now = Instant::now();
        let patience = Duration::from_secs(5);
        let mut pace = Pace::new(now, START, CEILING);

        let arrival = |caller: u64| now + Duration::from_millis(caller * 10);
        let granted = (0..2000)
            .filter(|caller| pace.reserve_within(arrival(*caller), patience).is_ok())
            .count();

        let horizon = pace.next.saturating_duration_since(arrival(1999));
        assert!(granted > 0 && granted < 2000);
        assert!(horizon <= patience + Duration::from_secs_f64(1.0 / START));
    }
}
