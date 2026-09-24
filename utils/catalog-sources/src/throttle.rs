use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::Mutex;

pub struct Throttle {
    interval: Duration,
    last: Mutex<Option<Instant>>,
}

impl Throttle {
    pub fn new(interval: Duration) -> Arc<Self> {
        Arc::new(Self {
            interval,
            last: Mutex::new(None),
        })
    }

    pub async fn wait(&self) {
        let now = Instant::now();
        let slot = {
            let mut g = self.last.lock().await;
            let next = match *g {
                Some(t) => (t + self.interval).max(now),
                None => now,
            };
            *g = Some(next);
            next
        };
        if let Some(delay) = slot.checked_duration_since(Instant::now())
            && !delay.is_zero()
        {
            tokio::time::sleep(delay).await;
        }
    }
}
