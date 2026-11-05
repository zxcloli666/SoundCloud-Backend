use std::sync::{Mutex, PoisonError};
use std::time::Instant;

pub struct Bucket {
    per_second: f64,
    burst: f64,
    state: Mutex<(Instant, f64)>,
}

impl Bucket {
    pub fn new(per_min: u32, burst: u32) -> Self {
        let per_min = per_min.max(1);
        let burst = f64::from(burst.min(per_min).max(1));
        Self {
            per_second: f64::from(per_min) / 60.0,
            burst,
            state: Mutex::new((Instant::now(), burst)),
        }
    }

    pub fn take(&self, now: Instant) -> Result<(), u64> {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        let (at, tokens) = &mut *state;
        let elapsed = now.saturating_duration_since(*at).as_secs_f64();
        *tokens = (*tokens + elapsed * self.per_second).min(self.burst);
        *at = now.max(*at);
        if *tokens >= 1.0 {
            *tokens -= 1.0;
            return Ok(());
        }
        let wait_ms = ((1.0 - *tokens) / self.per_second * 1000.0).ceil();
        Err(wait_ms as u64)
    }
}
