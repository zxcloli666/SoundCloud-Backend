use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tokio::sync::{OwnedSemaphorePermit, Semaphore};

pub(crate) const BUSY_RETRY_AFTER: Duration = Duration::from_secs(30);
const MISS_KEPT_FOR: Duration = Duration::from_secs(60 * 60);
const MISSES_AT_MOST: usize = 50_000;

pub struct FreeHq {
    streams: Arc<Semaphore>,
    downloads: Arc<Semaphore>,
    misses: Mutex<HashMap<String, Instant>>,
}

pub(crate) enum Gate {
    Open(Option<Arc<OwnedSemaphorePermit>>),
    Busy,
    MissedLately,
}

impl FreeHq {
    pub fn new(fetches: usize) -> Self {
        let fetches = fetches.min(Semaphore::MAX_PERMITS);
        Self {
            streams: Arc::new(Semaphore::new(fetches)),
            downloads: Arc::new(Semaphore::new(fetches)),
            misses: Mutex::new(HashMap::new()),
        }
    }

    pub(crate) fn gate(&self, track_urn: &str, is_premium: bool) -> Gate {
        if is_premium {
            return Gate::Open(None);
        }
        if self.missed_lately(track_urn, Instant::now()) {
            return Gate::MissedLately;
        }
        match self.streams.clone().try_acquire_owned() {
            Ok(permit) => Gate::Open(Some(Arc::new(permit))),
            Err(_) => Gate::Busy,
        }
    }

    pub(crate) fn download_slot(&self) -> Option<OwnedSemaphorePermit> {
        self.downloads.clone().try_acquire_owned().ok()
    }

    pub(crate) fn note_miss(&self, track_urn: &str) {
        self.note_miss_at(track_urn, Instant::now());
    }

    fn missed_lately(&self, track_urn: &str, now: Instant) -> bool {
        let misses = self.misses.lock().unwrap_or_else(|e| e.into_inner());
        misses
            .get(track_urn)
            .is_some_and(|at| now.duration_since(*at) < MISS_KEPT_FOR)
    }

    fn note_miss_at(&self, track_urn: &str, now: Instant) {
        let mut misses = self.misses.lock().unwrap_or_else(|e| e.into_inner());
        if misses.len() >= MISSES_AT_MOST {
            misses.retain(|_, at| now.duration_since(*at) < MISS_KEPT_FOR);
        }
        if misses.len() < MISSES_AT_MOST {
            misses.insert(track_urn.to_owned(), now);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const URN: &str = "soundcloud:tracks:1";

    #[test]
    fn star_never_waits_for_a_free_slot() {
        assert!(matches!(FreeHq::new(0).gate(URN, true), Gate::Open(None)));
    }

    #[test]
    fn with_no_free_slots_a_free_session_is_turned_away_before_any_request() {
        assert!(matches!(FreeHq::new(0).gate(URN, false), Gate::Busy));
        assert!(FreeHq::new(0).download_slot().is_none());
    }

    #[test]
    fn a_free_slot_comes_back_when_its_holder_lets_go() {
        let free = FreeHq::new(1);
        let Gate::Open(Some(held)) = free.gate(URN, false) else {
            panic!("the only slot was free");
        };
        assert!(matches!(free.gate(URN, false), Gate::Busy));
        drop(held);
        assert!(matches!(free.gate(URN, false), Gate::Open(Some(_))));
    }

    #[test]
    fn streams_and_downloads_do_not_eat_each_other_slots() {
        let free = FreeHq::new(1);
        let _download = free.download_slot().expect("the download slot was free");
        assert!(matches!(free.gate(URN, false), Gate::Open(Some(_))));
        assert!(free.download_slot().is_none());
    }

    #[test]
    fn a_track_that_had_no_hq_is_skipped_for_a_while_and_then_tried_again() {
        let free = FreeHq::new(4);
        free.note_miss(URN);

        assert!(matches!(free.gate(URN, false), Gate::MissedLately));
        assert!(matches!(free.gate(URN, true), Gate::Open(None)));
        assert!(matches!(
            free.gate("soundcloud:tracks:2", false),
            Gate::Open(Some(_))
        ));

        let later = Instant::now() + MISS_KEPT_FOR;
        assert!(!free.missed_lately(URN, later));
    }

    #[test]
    fn the_miss_list_stops_growing_at_its_cap() {
        let free = FreeHq::new(1);
        let now = Instant::now();
        for id in 0..MISSES_AT_MOST + 10 {
            free.note_miss_at(&format!("soundcloud:tracks:{id}"), now);
        }
        assert_eq!(free.misses.lock().unwrap().len(), MISSES_AT_MOST);

        let later = now + MISS_KEPT_FOR;
        free.note_miss_at("soundcloud:tracks:fresh", later);
        assert_eq!(free.misses.lock().unwrap().len(), 1);
    }
}
