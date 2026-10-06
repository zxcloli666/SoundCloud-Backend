use std::sync::atomic::{AtomicI64, AtomicU32, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::RelayRead;

const BAN_THRESHOLD: u32 = 4;
pub(crate) const COOLDOWN: Duration = Duration::from_secs(60);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Trip {
    Steady,
    Opened,
    Closed,
}

#[derive(Default)]
pub struct ChannelHealth {
    consecutive_bans: AtomicU32,
    open_until_ms: AtomicI64,
}

impl ChannelHealth {
    pub fn is_open(&self) -> bool {
        now_ms() < self.open_until_ms.load(Ordering::Acquire)
    }

    pub fn record_ok(&self) -> Trip {
        self.consecutive_bans.store(0, Ordering::Release);
        if self.open_until_ms.swap(0, Ordering::AcqRel) > now_ms() {
            Trip::Closed
        } else {
            Trip::Steady
        }
    }

    pub fn record_ban(&self) -> Trip {
        let seen = self.consecutive_bans.fetch_add(1, Ordering::AcqRel) + 1;
        if seen < BAN_THRESHOLD {
            return Trip::Steady;
        }
        let now = now_ms();
        let previous = self
            .open_until_ms
            .swap(now + COOLDOWN.as_millis() as i64, Ordering::AcqRel);
        if previous > now {
            Trip::Steady
        } else {
            Trip::Opened
        }
    }

    pub fn observe<T>(&self, read: &RelayRead<T>) -> Trip {
        if read.is_unavailable() {
            self.record_ban()
        } else {
            self.record_ok()
        }
    }
}

pub(crate) fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|since| since.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_breaker_opens_at_the_threshold_and_a_success_resets_it() {
        let health = ChannelHealth::default();
        for _ in 0..BAN_THRESHOLD - 1 {
            health.record_ban();
            assert!(!health.is_open());
        }
        health.record_ban();
        assert!(health.is_open(), "the breaker must open at the threshold");
        health.record_ok();
        assert!(!health.is_open(), "a success must reset the breaker");
    }

    #[test]
    fn a_missing_entity_is_an_answer_and_only_silence_opens_the_breaker() {
        let health = ChannelHealth::default();
        for _ in 0..8 {
            assert_eq!(health.observe(&RelayRead::<()>::Missing), Trip::Steady);
        }
        assert!(!health.is_open());

        for _ in 0..8 {
            health.observe(&RelayRead::<()>::Unavailable);
        }
        assert!(health.is_open());
    }

    #[test]
    fn only_the_crossing_reports_a_trip_so_an_outage_is_announced_once() {
        let health = ChannelHealth::default();
        for _ in 0..BAN_THRESHOLD - 1 {
            assert_eq!(health.record_ban(), Trip::Steady);
        }
        assert_eq!(health.record_ban(), Trip::Opened);
        for _ in 0..16 {
            assert_eq!(
                health.record_ban(),
                Trip::Steady,
                "an already open breaker must not announce itself again"
            );
        }
        assert_eq!(health.record_ok(), Trip::Closed);
        assert_eq!(
            health.record_ok(),
            Trip::Steady,
            "a success on a closed breaker announces nothing"
        );
    }
}
