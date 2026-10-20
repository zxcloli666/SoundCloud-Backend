use std::collections::HashSet;
use std::sync::{Arc, Mutex, MutexGuard};

pub struct UploadSlots {
    size: usize,
    running: Arc<Mutex<HashSet<String>>>,
}

pub struct UploadSlot {
    track_urn: String,
    running: Arc<Mutex<HashSet<String>>>,
}

impl UploadSlots {
    pub fn new(size: usize) -> Self {
        Self {
            size,
            running: Arc::default(),
        }
    }

    pub fn half_free(&self) -> bool {
        lock(&self.running).len() * 2 <= self.size
    }

    pub fn take(&self, track_urn: &str) -> Option<UploadSlot> {
        let mut running = lock(&self.running);
        if running.len() >= self.size || !running.insert(track_urn.to_owned()) {
            return None;
        }
        Some(UploadSlot {
            track_urn: track_urn.to_owned(),
            running: self.running.clone(),
        })
    }
}

impl Drop for UploadSlot {
    fn drop(&mut self) {
        lock(&self.running).remove(&self.track_urn);
    }
}

fn lock(running: &Mutex<HashSet<String>>) -> MutexGuard<'_, HashSet<String>> {
    running.lock().unwrap_or_else(|e| e.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    const URN: &str = "soundcloud:tracks:1";

    #[test]
    fn a_storage_that_never_answers_holds_at_most_the_slot_count_of_tracks() {
        let slots = UploadSlots::new(8);
        let held: Vec<UploadSlot> = (0..100)
            .filter_map(|id| slots.take(&format!("soundcloud:tracks:{id}")))
            .collect();

        assert_eq!(held.len(), 8, "92 of 100 uploads had to be skipped");
        assert!(!slots.half_free());

        drop(held);
        assert!(slots.half_free());
        assert!(
            lock(&slots.running).is_empty(),
            "a finished upload must not leave its track behind"
        );
    }

    #[test]
    fn a_track_that_is_uploading_is_not_uploaded_again_until_it_ends() {
        let slots = UploadSlots::new(8);
        let first = slots.take(URN).expect("every slot was free");

        assert!(slots.take(URN).is_none());
        assert!(slots.take("soundcloud:tracks:2").is_some());

        drop(first);
        assert!(slots.take(URN).is_some());
    }

    #[test]
    fn a_track_turned_away_as_a_duplicate_takes_no_slot() {
        let slots = UploadSlots::new(2);
        let _first = slots.take(URN).expect("every slot was free");
        assert!(slots.take(URN).is_none());

        let second = slots.take("soundcloud:tracks:2");
        assert!(
            second.is_some(),
            "the duplicate must not have used up the last slot"
        );
        assert!(!slots.half_free());
    }

    #[test]
    fn background_work_stops_once_played_tracks_hold_half_the_slots() {
        let slots = UploadSlots::new(8);
        let mut held: Vec<UploadSlot> = (0..4)
            .filter_map(|id| slots.take(&format!("soundcloud:tracks:{id}")))
            .collect();
        assert!(slots.half_free());

        held.extend(slots.take("soundcloud:tracks:4"));
        assert!(!slots.half_free());
        assert!(
            slots.take("soundcloud:tracks:5").is_some(),
            "a played track still gets one of the slots background work leaves alone"
        );
    }
}
