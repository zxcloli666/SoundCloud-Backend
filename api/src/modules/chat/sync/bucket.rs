use std::time::Duration;

use tokio::time::Instant;

pub const IMMEDIATE_ANSWERS: f64 = 20.0;
pub const REFILL_WINDOW: Duration = Duration::from_secs(10);
pub const OVER_BUDGET_WAIT: Duration = Duration::from_secs(1);

#[derive(Clone, Copy, Debug)]
pub struct Bucket {
    tokens: f64,
    at: Instant,
}

impl Bucket {
    pub fn full(now: Instant) -> Self {
        Self {
            tokens: IMMEDIATE_ANSWERS,
            at: now,
        }
    }

    pub fn take(&mut self, now: Instant) -> bool {
        let refill = now.saturating_duration_since(self.at).as_secs_f64() * IMMEDIATE_ANSWERS
            / REFILL_WINDOW.as_secs_f64();
        self.tokens = (self.tokens + refill).min(IMMEDIATE_ANSWERS);
        self.at = now;
        if self.tokens < 1.0 {
            return false;
        }
        self.tokens -= 1.0;
        true
    }

    pub fn is_full(&self, now: Instant) -> bool {
        now.saturating_duration_since(self.at) >= REFILL_WINDOW
    }
}
