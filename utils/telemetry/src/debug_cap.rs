use std::sync::Mutex;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

const DEFAULT_PER_SECOND: f64 = 10.0;
const BURST_SECONDS: f64 = 3.0;

static CAPPED: AtomicU64 = AtomicU64::new(0);
static BUCKET: OnceLock<Mutex<Bucket>> = OnceLock::new();

pub(crate) struct Bucket {
    rate: f64,
    burst: f64,
    tokens: f64,
    seen: Instant,
}

impl Bucket {
    pub(crate) fn new(rate: f64, now: Instant) -> Self {
        let burst = (rate * BURST_SECONDS).max(1.0);
        Self {
            rate,
            burst,
            tokens: burst,
            seen: now,
        }
    }

    pub(crate) fn take(&mut self, now: Instant) -> bool {
        let elapsed = now.saturating_duration_since(self.seen).as_secs_f64();
        self.seen = now;
        self.tokens = (self.tokens + elapsed * self.rate).min(self.burst);
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            true
        } else {
            false
        }
    }
}

fn configured() -> f64 {
    std::env::var("TELEMETRY_DEBUG_ROOTS_PER_SECOND")
        .ok()
        .and_then(|value| value.trim().parse::<f64>().ok())
        .filter(|rate| rate.is_finite() && *rate >= 0.0)
        .unwrap_or(DEFAULT_PER_SECOND)
}

pub(crate) fn admit() -> bool {
    let bucket = BUCKET.get_or_init(|| Mutex::new(Bucket::new(configured(), Instant::now())));
    let admitted = bucket
        .lock()
        .map(|mut bucket| bucket.take(Instant::now()))
        .unwrap_or(false);
    if !admitted {
        CAPPED.fetch_add(1, Ordering::Relaxed);
    }
    admitted
}

pub fn capped_total() -> u64 {
    CAPPED.load(Ordering::Relaxed)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[test]
    fn a_burst_passes_then_the_bucket_refills_at_the_rate() {
        let start = Instant::now();
        let mut bucket = Bucket::new(2.0, start);

        let burst = (0..10).filter(|_| bucket.take(start)).count();
        let later = start + Duration::from_secs(1);
        let refilled = (0..10).filter(|_| bucket.take(later)).count();

        assert_eq!(burst, 6);
        assert_eq!(refilled, 2);
    }

    #[test]
    fn a_zero_rate_admits_nothing_after_its_single_token() {
        let start = Instant::now();
        let mut bucket = Bucket::new(0.0, start);

        assert!(bucket.take(start));
        assert!(!bucket.take(start + Duration::from_secs(60)));
    }
}
