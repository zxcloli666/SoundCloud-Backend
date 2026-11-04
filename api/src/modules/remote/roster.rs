use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use backend_contracts::remote::MAX_DEVICES_PER_USER;

pub const DEVICE_TTL: Duration = Duration::from_secs(45);
pub const POLL_TTL: Duration = Duration::from_secs(60);
const SWEEP_EVERY: Duration = Duration::from_secs(30);

struct Seen {
    device: String,
    active: i64,
    at: Instant,
}

struct Devices {
    by_user: HashMap<String, Vec<Seen>>,
    swept_at: Option<Instant>,
}

struct Polls {
    by_user: HashMap<String, Instant>,
    swept_at: Option<Instant>,
}

pub struct Roster {
    devices: Mutex<Devices>,
    polls: Mutex<Polls>,
}

impl Default for Roster {
    fn default() -> Self {
        Self {
            devices: Mutex::new(Devices {
                by_user: HashMap::new(),
                swept_at: None,
            }),
            polls: Mutex::new(Polls {
                by_user: HashMap::new(),
                swept_at: None,
            }),
        }
    }
}

fn sweep_due(swept_at: &mut Option<Instant>, now: Instant) -> bool {
    match swept_at {
        Some(at) if now.duration_since(*at) < SWEEP_EVERY => false,
        _ => {
            *swept_at = Some(now);
            true
        }
    }
}

impl Roster {
    pub fn note(&self, user: &str, device: &str, active: i64, now: Instant) {
        let mut devices = self.devices.lock().unwrap_or_else(|e| e.into_inner());
        if sweep_due(&mut devices.swept_at, now) {
            devices
                .by_user
                .retain(|_, seen| seen.iter().any(|s| now.duration_since(s.at) < DEVICE_TTL));
        }
        let seen = devices.by_user.entry(user.to_owned()).or_default();
        seen.retain(|s| now.duration_since(s.at) < DEVICE_TTL && s.device != device);
        if seen.len() >= MAX_DEVICES_PER_USER
            && let Some(oldest) = seen
                .iter()
                .enumerate()
                .min_by_key(|(_, s)| s.at)
                .map(|(index, _)| index)
        {
            seen.swap_remove(oldest);
        }
        seen.push(Seen {
            device: device.to_owned(),
            active,
            at: now,
        });
    }

    pub fn winner(&self, user: &str, now: Instant) -> Option<String> {
        let devices = self.devices.lock().unwrap_or_else(|e| e.into_inner());
        devices
            .by_user
            .get(user)?
            .iter()
            .filter(|s| now.duration_since(s.at) < DEVICE_TTL)
            .max_by(|a, b| (a.active, &a.device).cmp(&(b.active, &b.device)))
            .map(|s| s.device.clone())
    }

    pub fn is_target(&self, user: &str, device: &str, now: Instant) -> bool {
        match self.winner(user, now) {
            Some(winner) => winner == device,
            None => true,
        }
    }

    pub fn mark_polled(&self, user: &str, now: Instant) {
        let mut polls = self.polls.lock().unwrap_or_else(|e| e.into_inner());
        if sweep_due(&mut polls.swept_at, now) {
            polls
                .by_user
                .retain(|_, at| now.duration_since(*at) < POLL_TTL);
        }
        polls.by_user.insert(user.to_owned(), now);
    }

    pub fn polled_recently(&self, user: &str, now: Instant) -> bool {
        let polls = self.polls.lock().unwrap_or_else(|e| e.into_inner());
        polls
            .by_user
            .get(user)
            .is_some_and(|at| now.duration_since(*at) < POLL_TTL)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_device_active_most_recently_wins() {
        let roster = Roster::default();
        let now = Instant::now();
        roster.note("1", "laptop", 100, now);
        roster.note("1", "desktop", 200, now);
        assert_eq!(roster.winner("1", now).as_deref(), Some("desktop"));
        assert!(roster.is_target("1", "desktop", now));
        assert!(!roster.is_target("1", "laptop", now));
        roster.note("1", "laptop", 300, now);
        assert_eq!(roster.winner("1", now).as_deref(), Some("laptop"));
    }

    #[test]
    fn a_silent_device_drops_out() {
        let roster = Roster::default();
        let start = Instant::now();
        roster.note("1", "old", 900, start);
        let later = start + DEVICE_TTL + Duration::from_secs(1);
        roster.note("1", "new", 100, later);
        assert_eq!(roster.winner("1", later).as_deref(), Some("new"));
        assert_eq!(roster.winner("1", later + DEVICE_TTL * 2), None);
    }

    #[test]
    fn an_unknown_user_accepts_any_device() {
        let roster = Roster::default();
        assert!(roster.is_target("9", "any", Instant::now()));
    }

    #[test]
    fn a_user_keeps_at_most_three_devices_and_the_stalest_goes_first() {
        let roster = Roster::default();
        let start = Instant::now();
        for (index, device) in ["a", "b", "c", "d"].into_iter().enumerate() {
            roster.note(
                "1",
                device,
                index as i64,
                start + Duration::from_secs(index as u64),
            );
        }
        let now = start + Duration::from_secs(4);
        assert_eq!(roster.winner("1", now).as_deref(), Some("d"));
        let devices = roster.devices.lock().unwrap();
        let names: Vec<&str> = devices.by_user["1"]
            .iter()
            .map(|s| s.device.as_str())
            .collect();
        assert_eq!(names.len(), 3);
        assert!(!names.contains(&"a"));
    }

    #[test]
    fn expired_entries_are_swept_at_most_every_half_minute() {
        let roster = Roster::default();
        let start = Instant::now();
        let seconds = Duration::from_secs;
        roster.note("old", "pc", 1, start);
        roster.note("a", "pc", 1, start + seconds(50));
        assert!(!roster.devices.lock().unwrap().by_user.contains_key("old"));
        roster.note("stale", "pc", 1, start);
        roster.note("b", "pc", 1, start + seconds(60));
        assert!(roster.devices.lock().unwrap().by_user.contains_key("stale"));
        roster.note("c", "pc", 1, start + seconds(81));
        assert!(!roster.devices.lock().unwrap().by_user.contains_key("stale"));
    }

    #[test]
    fn a_poll_counts_for_a_minute() {
        let roster = Roster::default();
        let start = Instant::now();
        assert!(!roster.polled_recently("1", start));
        roster.mark_polled("1", start);
        assert!(roster.polled_recently("1", start + Duration::from_secs(59)));
        assert!(!roster.polled_recently("1", start + Duration::from_secs(61)));
    }
}
