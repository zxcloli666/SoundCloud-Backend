use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub(crate) const BUSY_RETRY_AFTER: Duration = Duration::from_secs(30);
const SLOTS_PER_SESSION: usize = 2;
const MISS_KEPT_FOR: Duration = Duration::from_secs(15 * 60);
const MISS_SWEEP_EVERY: Duration = Duration::from_secs(60);
const MISSES_AT_MOST: usize = 50_000;

pub struct FreeHq {
    streams: Pool,
    downloads: Pool,
    misses: Mutex<Misses>,
}

pub(crate) enum Gate {
    Open(Option<Arc<FreeSlot>>),
    Busy,
    MissedLately,
}

#[derive(Debug)]
pub(crate) struct FreeSlot {
    session: String,
    held: Arc<Mutex<Held>>,
}

struct Pool {
    size: usize,
    held: Arc<Mutex<Held>>,
}

#[derive(Debug, Default)]
struct Held {
    total: usize,
    by_session: HashMap<String, usize>,
}

#[derive(Default)]
struct Misses {
    at: HashMap<String, Instant>,
    swept: Option<Instant>,
}

impl FreeHq {
    pub fn new(fetches: usize) -> Self {
        Self {
            streams: Pool::new(fetches),
            downloads: Pool::new(fetches),
            misses: Mutex::default(),
        }
    }

    pub(crate) fn gate(&self, track_urn: &str, session: &str, is_premium: bool) -> Gate {
        if is_premium {
            return Gate::Open(None);
        }
        self.open(&self.streams, track_urn, session)
    }

    pub(crate) fn download_gate(&self, track_urn: &str, session: &str) -> Gate {
        self.open(&self.downloads, track_urn, session)
    }

    pub(crate) fn download_slot(&self, session: &str) -> Option<Arc<FreeSlot>> {
        self.downloads.take(session)
    }

    pub(crate) fn note_miss(&self, track_urn: &str) {
        self.note_miss_at(track_urn, Instant::now());
    }

    fn open(&self, pool: &Pool, track_urn: &str, session: &str) -> Gate {
        if self.missed_lately(track_urn, Instant::now()) {
            return Gate::MissedLately;
        }
        match pool.take(session) {
            Some(slot) => Gate::Open(Some(slot)),
            None => Gate::Busy,
        }
    }

    fn missed_lately(&self, track_urn: &str, now: Instant) -> bool {
        let misses = self.misses.lock().unwrap_or_else(|e| e.into_inner());
        misses
            .at
            .get(track_urn)
            .is_some_and(|at| now.duration_since(*at) < MISS_KEPT_FOR)
    }

    fn note_miss_at(&self, track_urn: &str, now: Instant) {
        let mut misses = self.misses.lock().unwrap_or_else(|e| e.into_inner());
        let sweep_due = misses
            .swept
            .is_none_or(|swept| now.duration_since(swept) >= MISS_SWEEP_EVERY);
        if misses.at.len() >= MISSES_AT_MOST && sweep_due {
            misses
                .at
                .retain(|_, at| now.duration_since(*at) < MISS_KEPT_FOR);
            misses.swept = Some(now);
        }
        if misses.at.len() < MISSES_AT_MOST {
            misses.at.insert(track_urn.to_owned(), now);
        }
    }
}

impl Pool {
    fn new(size: usize) -> Self {
        Self {
            size,
            held: Arc::default(),
        }
    }

    fn take(&self, session: &str) -> Option<Arc<FreeSlot>> {
        let mut held = self.held.lock().unwrap_or_else(|e| e.into_inner());
        let mine = held.by_session.get(session).copied().unwrap_or(0);
        if held.total >= self.size || mine >= SLOTS_PER_SESSION {
            return None;
        }
        held.total += 1;
        held.by_session.insert(session.to_owned(), mine + 1);
        Some(Arc::new(FreeSlot {
            session: session.to_owned(),
            held: self.held.clone(),
        }))
    }
}

impl Drop for FreeSlot {
    fn drop(&mut self) {
        let mut held = self.held.lock().unwrap_or_else(|e| e.into_inner());
        held.total -= 1;
        if let Some(mine) = held.by_session.get_mut(&self.session) {
            *mine -= 1;
            if *mine == 0 {
                held.by_session.remove(&self.session);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const URN: &str = "soundcloud:tracks:1";
    const ME: &str = "session-me";
    const YOU: &str = "session-you";

    #[test]
    fn star_never_waits_for_a_free_slot() {
        assert!(matches!(
            FreeHq::new(0).gate(URN, ME, true),
            Gate::Open(None)
        ));
    }

    #[test]
    fn with_no_free_slots_a_free_session_is_turned_away_before_any_request() {
        let free = FreeHq::new(0);
        assert!(matches!(free.gate(URN, ME, false), Gate::Busy));
        assert!(matches!(free.download_gate(URN, ME), Gate::Busy));
        assert!(free.download_slot(ME).is_none());
    }

    #[test]
    fn a_free_slot_comes_back_when_its_holder_lets_go() {
        let free = FreeHq::new(1);
        let Gate::Open(Some(held)) = free.gate(URN, ME, false) else {
            panic!("the only slot was free");
        };
        assert!(matches!(free.gate(URN, YOU, false), Gate::Busy));
        drop(held);
        assert!(matches!(free.gate(URN, YOU, false), Gate::Open(Some(_))));
    }

    #[test]
    fn streams_and_downloads_do_not_eat_each_other_slots() {
        let free = FreeHq::new(1);
        let _download = free.download_slot(ME).expect("the download slot was free");
        assert!(matches!(free.gate(URN, ME, false), Gate::Open(Some(_))));
        assert!(free.download_slot(YOU).is_none());
    }

    #[test]
    fn one_session_cannot_hold_more_than_its_share_of_free_slots() {
        let free = FreeHq::new(8);
        let mine: Vec<_> = (0..SLOTS_PER_SESSION)
            .map(|_| free.download_slot(ME).expect("a share is free"))
            .collect();

        assert!(free.download_slot(ME).is_none());
        assert!(matches!(free.download_gate(URN, ME), Gate::Busy));
        assert!(
            free.download_slot(YOU).is_some(),
            "one busy client must not turn everybody else away"
        );
        assert!(
            matches!(free.gate(URN, ME, false), Gate::Open(Some(_))),
            "a play races /download against /stream, so each pool keeps its own share"
        );

        drop(mine);
        assert!(free.download_slot(ME).is_some());
        assert!(
            free.downloads.held.lock().unwrap().by_session.is_empty(),
            "a session that let go of every slot leaves nothing behind"
        );
    }

    #[test]
    fn a_track_that_had_no_hq_is_skipped_for_a_while_and_then_tried_again() {
        let free = FreeHq::new(4);
        free.note_miss(URN);

        assert!(matches!(free.gate(URN, ME, false), Gate::MissedLately));
        assert!(matches!(free.download_gate(URN, ME), Gate::MissedLately));
        assert!(matches!(free.gate(URN, ME, true), Gate::Open(None)));
        assert!(matches!(
            free.gate("soundcloud:tracks:2", ME, false),
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
        assert_eq!(free.misses.lock().unwrap().at.len(), MISSES_AT_MOST);

        let later = now + MISS_KEPT_FOR;
        free.note_miss_at("soundcloud:tracks:fresh", later);
        assert_eq!(free.misses.lock().unwrap().at.len(), 1);
    }

    #[test]
    fn a_full_miss_list_is_swept_at_most_once_a_minute() {
        let free = FreeHq::new(1);
        let start = Instant::now();
        for id in 0..MISSES_AT_MOST {
            free.note_miss_at(&format!("soundcloud:tracks:{id}"), start);
        }
        let swept = start + MISS_KEPT_FOR - Duration::from_secs(10);
        free.note_miss_at("soundcloud:tracks:a", swept);

        free.note_miss_at("soundcloud:tracks:b", swept + Duration::from_secs(20));
        assert_eq!(
            free.misses.lock().unwrap().at.len(),
            MISSES_AT_MOST,
            "a sweep holds the lock every gate takes; one right after another is wasted"
        );

        free.note_miss_at("soundcloud:tracks:c", swept + MISS_SWEEP_EVERY);
        assert_eq!(free.misses.lock().unwrap().at.len(), 1);
    }
}
