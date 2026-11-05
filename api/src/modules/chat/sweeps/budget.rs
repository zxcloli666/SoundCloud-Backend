use std::time::Duration;

use tokio::time::Instant;

use crate::modules::chat::ChatState;

pub const STEP_TIME: Duration = Duration::from_secs(10);

pub struct Budget {
    batches: u32,
    until: Instant,
}

impl Budget {
    pub fn new(batches: u32) -> Self {
        Self::until(batches, Instant::now() + STEP_TIME)
    }

    pub fn until(batches: u32, until: Instant) -> Self {
        Self { batches, until }
    }

    pub fn in_time(&self, chat: &ChatState) -> bool {
        Instant::now() < self.until && !chat.shutdown.is_cancelled()
    }

    pub fn left(&self, chat: &ChatState) -> bool {
        self.batches > 0 && self.in_time(chat)
    }

    pub fn spend(&mut self, chat: &ChatState) -> bool {
        if !self.left(chat) {
            return false;
        }
        self.batches -= 1;
        true
    }
}
