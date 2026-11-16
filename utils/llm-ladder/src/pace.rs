use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::Duration;

use tokio::time::Instant;

const WINDOW: Duration = Duration::from_secs(60);

pub(crate) struct Pace {
    per_minute: usize,
    starts: Mutex<VecDeque<Instant>>,
}

impl Pace {
    pub fn per_minute(limit: u32) -> Self {
        Self {
            per_minute: limit as usize,
            starts: Mutex::default(),
        }
    }

    pub fn take(&self, now: Instant) -> bool {
        let mut starts = self
            .starts
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        while starts
            .front()
            .is_some_and(|at| now.duration_since(*at) >= WINDOW)
        {
            starts.pop_front();
        }
        if starts.len() >= self.per_minute {
            return false;
        }
        starts.push_back(now);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(start_paused = true)]
    async fn the_limit_holds_for_any_minute_and_frees_as_it_slides() {
        let pace = Pace::per_minute(3);
        assert!(pace.take(Instant::now()));
        tokio::time::advance(Duration::from_secs(20)).await;
        assert!(pace.take(Instant::now()));
        assert!(pace.take(Instant::now()));
        assert!(!pace.take(Instant::now()));
        tokio::time::advance(Duration::from_secs(39)).await;
        assert!(!pace.take(Instant::now()), "the first start is 59 s old");
        tokio::time::advance(Duration::from_secs(1)).await;
        assert!(pace.take(Instant::now()));
        assert!(!pace.take(Instant::now()));
        tokio::time::advance(Duration::from_secs(20)).await;
        assert!(pace.take(Instant::now()));
        assert!(pace.take(Instant::now()));
    }
}
