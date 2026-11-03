use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

pub const DEVICE_TTL: Duration = Duration::from_secs(45);
pub const POLL_TTL: Duration = Duration::from_secs(60);

struct Seen {
    device: String,
    active: i64,
    at: Instant,
}

#[derive(Default)]
pub struct Roster {
    devices: Mutex<HashMap<String, Vec<Seen>>>,
    polls: Mutex<HashMap<String, Instant>>,
}

impl Roster {
    pub fn note(&self, user: &str, device: &str, active: i64, now: Instant) {
        let mut devices = self.devices.lock().unwrap_or_else(|e| e.into_inner());
        if devices.len() > 4096 {
            devices.retain(|_, seen| seen.iter().any(|s| now.duration_since(s.at) < DEVICE_TTL));
        }
        let seen = devices.entry(user.to_owned()).or_default();
        seen.retain(|s| now.duration_since(s.at) < DEVICE_TTL && s.device != device);
        seen.push(Seen {
            device: device.to_owned(),
            active,
            at: now,
        });
    }

    pub fn winner(&self, user: &str, now: Instant) -> Option<String> {
        let devices = self.devices.lock().unwrap_or_else(|e| e.into_inner());
        devices
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
        if polls.len() > 4096 {
            polls.retain(|_, at| now.duration_since(*at) < POLL_TTL);
        }
        polls.insert(user.to_owned(), now);
    }

    pub fn polled_recently(&self, user: &str, now: Instant) -> bool {
        let polls = self.polls.lock().unwrap_or_else(|e| e.into_inner());
        polls
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
    fn a_poll_counts_for_a_minute() {
        let roster = Roster::default();
        let start = Instant::now();
        assert!(!roster.polled_recently("1", start));
        roster.mark_polled("1", start);
        assert!(roster.polled_recently("1", start + Duration::from_secs(59)));
        assert!(!roster.polled_recently("1", start + Duration::from_secs(61)));
    }
}
