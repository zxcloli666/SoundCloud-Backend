use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use uuid::Uuid;

pub const RETRY_DELAYS: [Duration; 3] = [
    Duration::from_secs(60),
    Duration::from_secs(5 * 60),
    Duration::from_secs(30 * 60),
];
pub const FORGET_AFTER: Duration = Duration::from_secs(3_600);

#[derive(Default)]
pub struct Later {
    waits: Mutex<HashMap<Uuid, Wait>>,
}

struct Wait {
    tries: usize,
    until: Instant,
}

impl Later {
    pub fn postpone(&self, report: Uuid) -> Option<Duration> {
        let mut waits = self.waits();
        let tries = waits.get(&report).map_or(0, |wait| wait.tries);
        let Some(delay) = RETRY_DELAYS.get(tries).copied() else {
            waits.remove(&report);
            return None;
        };
        let wait = Wait {
            tries: tries + 1,
            until: Instant::now() + delay,
        };
        waits.insert(report, wait);
        Some(delay)
    }

    pub fn forget(&self, report: Uuid) {
        self.waits().remove(&report);
    }

    pub fn waiting(&self) -> Vec<Uuid> {
        let now = Instant::now();
        let mut waits = self.waits();
        waits.retain(|_, wait| now < wait.until + FORGET_AFTER);
        waits
            .iter()
            .filter(|(_, wait)| now < wait.until)
            .map(|(report, _)| *report)
            .collect()
    }

    fn waits(&self) -> MutexGuard<'_, HashMap<Uuid, Wait>> {
        self.waits.lock().unwrap_or_else(PoisonError::into_inner)
    }
}
