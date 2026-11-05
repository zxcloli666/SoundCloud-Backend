use backend_contracts::remote::MAX_DEVICES_PER_USER;
use serde::{Deserialize, Serialize};

pub const DEVICE_TTL_MS: i64 = 45_000;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Devices {
    #[serde(default)]
    pub seen: Vec<Seen>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Seen {
    pub device: String,
    pub active: i64,
    pub at_ms: i64,
}

fn fresh(seen: &Seen, now_ms: i64) -> bool {
    now_ms - seen.at_ms < DEVICE_TTL_MS
}

impl Devices {
    pub fn note(&mut self, device: &str, active: i64, now_ms: i64) {
        self.seen
            .retain(|seen| fresh(seen, now_ms) && seen.device != device);
        if self.seen.len() >= MAX_DEVICES_PER_USER
            && let Some(oldest) = self
                .seen
                .iter()
                .enumerate()
                .min_by_key(|(_, seen)| seen.at_ms)
                .map(|(index, _)| index)
        {
            self.seen.swap_remove(oldest);
        }
        self.seen.push(Seen {
            device: device.to_owned(),
            active,
            at_ms: now_ms,
        });
    }

    pub fn winner(&self, now_ms: i64) -> Option<&str> {
        self.seen
            .iter()
            .filter(|seen| fresh(seen, now_ms))
            .max_by(|a, b| (a.active, &a.device).cmp(&(b.active, &b.device)))
            .map(|seen| seen.device.as_str())
    }

    pub fn is_target(&self, device: &str, now_ms: i64) -> bool {
        self.winner(now_ms).is_none_or(|winner| winner == device)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_device_active_most_recently_wins() {
        let mut devices = Devices::default();
        devices.note("laptop", 100, 0);
        devices.note("desktop", 200, 0);
        assert_eq!(devices.winner(0), Some("desktop"));
        assert!(devices.is_target("desktop", 0));
        assert!(!devices.is_target("laptop", 0));
        devices.note("laptop", 300, 0);
        assert_eq!(devices.winner(0), Some("laptop"));
    }

    #[test]
    fn a_silent_device_drops_out() {
        let mut devices = Devices::default();
        devices.note("old", 900, 0);
        let later = DEVICE_TTL_MS + 1_000;
        devices.note("new", 100, later);
        assert_eq!(devices.winner(later), Some("new"));
        assert_eq!(devices.winner(later + DEVICE_TTL_MS * 2), None);
        assert_eq!(devices.seen.len(), 1);
    }

    #[test]
    fn an_unknown_user_accepts_any_device() {
        assert!(Devices::default().is_target("any", 0));
    }

    #[test]
    fn a_user_keeps_at_most_three_devices_and_the_stalest_goes_first() {
        let mut devices = Devices::default();
        for (index, device) in ["a", "b", "c", "d"].into_iter().enumerate() {
            devices.note(device, index as i64, index as i64 * 1_000);
        }
        assert_eq!(devices.winner(4_000), Some("d"));
        let names: Vec<&str> = devices.seen.iter().map(|s| s.device.as_str()).collect();
        assert_eq!(names.len(), 3);
        assert!(!names.contains(&"a"));
    }
}
