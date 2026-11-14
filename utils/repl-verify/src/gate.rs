use std::time::Duration;

const MEGABYTE: i64 = 1024 * 1024;
pub const HOLD_BACKLOG_BYTES: i64 = 64 * MEGABYTE;
pub const CALM_BACKLOG_BYTES: i64 = 8 * MEGABYTE;
pub const CATCH_UP_GAP: Duration = Duration::from_secs(2);
pub const MAX_BEHIND: Duration = Duration::from_secs(3600);
pub const MAX_HELD_TICKS: u32 = 120;

#[derive(Debug, Default)]
pub struct Gate {
    behind: Duration,
    held: u32,
    calm: bool,
}

impl Gate {
    pub fn hold(&mut self, backlog_bytes: i64, pace: Duration) -> bool {
        if backlog_bytes > HOLD_BACKLOG_BYTES && self.held < MAX_HELD_TICKS {
            self.held += 1;
            self.behind = (self.behind + pace).min(MAX_BEHIND);
            return true;
        }
        self.held = 0;
        self.calm = backlog_bytes < CALM_BACKLOG_BYTES;
        false
    }

    pub fn after_probe(&mut self, pace: Duration) -> Duration {
        if !self.calm || self.behind.is_zero() || pace <= CATCH_UP_GAP {
            return pace;
        }
        self.behind = self.behind.saturating_sub(pace - CATCH_UP_GAP);
        CATCH_UP_GAP
    }

    pub fn behind(&self) -> Duration {
        self.behind
    }
}
