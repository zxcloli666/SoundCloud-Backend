use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use sqlx::PgPool;
use tokio::time::MissedTickBehavior;
use tokio_util::sync::CancellationToken;

const FLOOR: usize = 2;
const CHECK_EVERY: Duration = Duration::from_secs(5);
const MEGABYTE: i64 = 1024 * 1024;
const CALM_BACKLOG_BYTES: i64 = 256 * MEGABYTE;
const HEAVY_BACKLOG_BYTES: i64 = 1024 * MEGABYTE;
const CALM_ACTIVE_BACKENDS: i64 = 16;
const HEAVY_ACTIVE_BACKENDS: i64 = 32;
const PRESSURE: &str = include_str!("../../queries/queue/write_pressure.sql");

pub(crate) struct WriteGovernor {
    limit: AtomicUsize,
    ceiling: usize,
}

#[derive(Clone, Copy, Debug, sqlx::FromRow)]
struct Pressure {
    replication_backlog_bytes: i64,
    active_backends: i64,
}

impl WriteGovernor {
    pub(crate) fn new(ceiling: usize) -> Arc<Self> {
        Arc::new(Self {
            limit: AtomicUsize::new(FLOOR.min(ceiling)),
            ceiling,
        })
    }

    pub(crate) fn limit(&self) -> usize {
        self.limit.load(Ordering::Relaxed)
    }

    pub(crate) async fn run(
        self: Arc<Self>,
        pool: PgPool,
        cancellation: CancellationToken,
    ) -> anyhow::Result<()> {
        let mut interval = tokio::time::interval(CHECK_EVERY);
        interval.set_missed_tick_behavior(MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                _ = cancellation.cancelled() => return Ok(()),
                _ = interval.tick() => {}
            }
            match sqlx::query_as::<_, Pressure>(PRESSURE)
                .fetch_one(&pool)
                .await
            {
                Ok(pressure) => self.adjust(pressure),
                Err(error) => {
                    tracing::warn!(%error, "database pressure is unknown, bulk concurrency stays");
                }
            }
        }
    }

    fn adjust(&self, pressure: Pressure) {
        let current = self.limit();
        let next = next_limit(current, self.ceiling, pressure);
        if next == current {
            return;
        }
        self.limit.store(next, Ordering::Relaxed);
        tracing::info!(
            from = current,
            to = next,
            replication_backlog_mb = pressure.replication_backlog_bytes / MEGABYTE,
            active_backends = pressure.active_backends,
            "bulk concurrency adjusted to the database pressure"
        );
    }
}

fn next_limit(current: usize, ceiling: usize, pressure: Pressure) -> usize {
    let floor = FLOOR.min(ceiling);
    let heavy = pressure.replication_backlog_bytes > HEAVY_BACKLOG_BYTES
        || pressure.active_backends > HEAVY_ACTIVE_BACKENDS;
    let calm = pressure.replication_backlog_bytes < CALM_BACKLOG_BYTES
        && pressure.active_backends < CALM_ACTIVE_BACKENDS;
    if heavy {
        (current / 2).max(floor)
    } else if calm {
        (current + 1).min(ceiling)
    } else {
        current
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pressure(backlog_mb: i64, active_backends: i64) -> Pressure {
        Pressure {
            replication_backlog_bytes: backlog_mb * MEGABYTE,
            active_backends,
        }
    }

    #[test]
    fn a_calm_database_gets_one_more_slot_up_to_the_ceiling() {
        assert_eq!(next_limit(4, 8, pressure(10, 3)), 5);
        assert_eq!(next_limit(8, 8, pressure(10, 3)), 8);
    }

    #[test]
    fn a_lagging_replica_or_a_busy_database_halves_the_slots_down_to_the_floor() {
        assert_eq!(next_limit(16, 32, pressure(2000, 3)), 8);
        assert_eq!(next_limit(16, 32, pressure(10, 40)), 8);
        assert_eq!(next_limit(3, 32, pressure(2000, 3)), 2);
        assert_eq!(next_limit(1, 1, pressure(2000, 3)), 1);
    }

    #[test]
    fn moderate_pressure_keeps_the_current_slots() {
        assert_eq!(next_limit(10, 32, pressure(600, 3)), 10);
        assert_eq!(next_limit(10, 32, pressure(10, 20)), 10);
    }
}
