use std::sync::Arc;
use std::time::Duration;

use sc_transport::EgressHealth;
use tokio::time::Instant;

pub const STALL_AFTER: Duration = Duration::from_secs(2);

pub struct StallWatch {
    health: Arc<EgressHealth>,
    channel: &'static str,
    started: Instant,
    done: bool,
}

impl StallWatch {
    pub fn start(health: &Arc<EgressHealth>, channel: &'static str) -> Self {
        Self {
            health: health.clone(),
            channel,
            started: Instant::now(),
            done: false,
        }
    }

    pub fn finish(mut self) {
        self.done = true;
    }
}

impl Drop for StallWatch {
    fn drop(&mut self) {
        if self.done || self.started.elapsed() < STALL_AFTER {
            return;
        }
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            return;
        };
        let health = self.health.clone();
        let channel = self.channel;
        runtime.spawn(async move {
            crate::metrics::set_relay_breaker_open(channel, health.record_stall().await);
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sc_transport::EGRESS_RELAY_LUA;

    async fn settle() {
        for _ in 0..8 {
            tokio::task::yield_now().await;
        }
    }

    #[tokio::test(start_paused = true)]
    async fn calls_dropped_after_the_stall_threshold_open_the_breaker() {
        let health = Arc::new(EgressHealth::new(EGRESS_RELAY_LUA, "api", None));
        for _ in 0..6 {
            let watch = StallWatch::start(&health, EGRESS_RELAY_LUA);
            tokio::time::advance(STALL_AFTER + Duration::from_millis(1)).await;
            drop(watch);
            settle().await;
        }
        assert!(health.is_open().await);
    }

    #[tokio::test(start_paused = true)]
    async fn quick_drops_and_finished_calls_leave_the_breaker_closed() {
        let health = Arc::new(EgressHealth::new(EGRESS_RELAY_LUA, "api", None));
        for _ in 0..12 {
            drop(StallWatch::start(&health, EGRESS_RELAY_LUA));
            let slow = StallWatch::start(&health, EGRESS_RELAY_LUA);
            tokio::time::advance(STALL_AFTER + Duration::from_millis(1)).await;
            slow.finish();
            settle().await;
        }
        assert!(!health.is_open().await);
    }
}
