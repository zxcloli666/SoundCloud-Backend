use backend_contracts::worker_contract::WorkerLaneSpec;
use sqlx::PgPool;

use crate::bus::Bus;
use crate::queue::{JobError, JobResult};

#[derive(Clone)]
pub struct WorkerBacklog {
    bus: Bus,
}

impl WorkerBacklog {
    pub fn new(bus: Bus) -> Self {
        Self { bus }
    }

    pub async fn room(&self, spec: &WorkerLaneSpec, target: i64) -> i64 {
        match self.bus.worker_pending(spec).await {
            Ok(pending) => room(target, pending),
            Err(error) => {
                tracing::warn!(
                    lane = spec.lane.as_str(),
                    %error,
                    "worker backlog is unknown; nothing new is dispatched this round"
                );
                0
            }
        }
    }
}

pub fn room(target: i64, pending: u64) -> i64 {
    let pending = i64::try_from(pending).unwrap_or(i64::MAX);
    target.saturating_sub(pending).max(0)
}

pub async fn unclaimed_room(pool: &PgPool, kind: &str, room: i64) -> JobResult<i64> {
    if room <= 0 {
        return Ok(0);
    }
    let queued = sqlx::query_file_scalar!("queries/queue/count_queued_kind.sql", kind, room)
        .fetch_one(pool)
        .await
        .map_err(JobError::retryable)?;
    Ok(room.saturating_sub(queued).max(0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_backlog_is_topped_up_only_to_its_target() {
        assert_eq!(room(256, 0), 256);
        assert_eq!(room(256, 200), 56);
        assert_eq!(room(256, 256), 0);
        assert_eq!(room(256, 10_000), 0);
        assert_eq!(room(256, u64::MAX), 0);
    }
}
