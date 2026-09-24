use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use backend_contracts::{COLLAB_MAX_MIN_COUNT, CollabTrainPayload, JobKind};
use deadpool_redis::Pool;
use deadpool_redis::redis::AsyncCommands;
use mini_moka::sync::Cache;
use tracing::{info, warn};
use uuid::Uuid;

use crate::config::CollabTriggerCfg;
use crate::error::{AppError, AppResult};

use super::{BackgroundJob, BackgroundJobs};

const DEDUP_KEY: &str = "schedule";
const SHARED_COUNTER_KEY: &str = "collab:trigger:events";
const SHARED_COOLDOWN_KEY: &str = "collab:trigger:cooldown";

pub struct CollabJobs {
    background_jobs: Arc<BackgroundJobs>,
    redis: Pool,
    event_threshold: u64,
    cooldown_secs: u64,
    recent_enqueue: Cache<(), ()>,
    event_count: AtomicU64,
    enqueueing: AtomicBool,
}

impl CollabJobs {
    pub fn new(
        background_jobs: Arc<BackgroundJobs>,
        redis: Pool,
        config: &CollabTriggerCfg,
    ) -> Arc<Self> {
        Arc::new(Self {
            background_jobs,
            redis,
            event_threshold: config.event_threshold,
            cooldown_secs: config.cooldown.as_secs().max(1),
            recent_enqueue: Cache::builder()
                .max_capacity(1)
                .time_to_live(config.cooldown)
                .build(),
            event_count: AtomicU64::new(0),
            enqueueing: AtomicBool::new(false),
        })
    }

    async fn claim_shared_trigger(&self) -> Option<bool> {
        let mut conn = self.redis.get().await.ok()?;
        let count: i64 = conn.incr(SHARED_COUNTER_KEY, 1).await.ok()?;
        if (count as u64) < self.event_threshold {
            return Some(false);
        }
        let claimed: Option<String> = deadpool_redis::redis::cmd("SET")
            .arg(SHARED_COOLDOWN_KEY)
            .arg(1)
            .arg("NX")
            .arg("EX")
            .arg(self.cooldown_secs)
            .query_async(&mut conn)
            .await
            .ok()?;
        if claimed.is_none() {
            return Some(false);
        }
        let _: Result<i64, _> = conn
            .decr(SHARED_COUNTER_KEY, self.event_threshold as i64)
            .await;
        Some(true)
    }

    pub async fn enqueue(&self, payload: CollabTrainPayload) -> AppResult<Uuid> {
        validate_payload(payload)?;
        let job = BackgroundJob::coalescing(JobKind::CollabTrain, DEDUP_KEY, payload)?;
        self.background_jobs.enqueue(&job).await
    }

    pub async fn note_event(&self) {
        if let Some(claimed) = self.claim_shared_trigger().await {
            if !claimed {
                return;
            }
            match self.enqueue(CollabTrainPayload::default()).await {
                Ok(job_id) => info!(%job_id, "collab training queued from user activity"),
                Err(error) => warn!(%error, "collab training enqueue failed"),
            }
            return;
        }

        let count = self.event_count.fetch_add(1, Ordering::Relaxed) + 1;
        if count < self.event_threshold || self.recent_enqueue.get(&()).is_some() {
            return;
        }
        if self
            .enqueueing
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return;
        }
        let _guard = EnqueueGuard(&self.enqueueing);
        if self.recent_enqueue.get(&()).is_some() {
            return;
        }

        match self.enqueue(CollabTrainPayload::default()).await {
            Ok(job_id) => {
                self.recent_enqueue.insert((), ());
                let _ =
                    self.event_count
                        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |count| {
                            Some(count.saturating_sub(self.event_threshold))
                        });
                info!(%job_id, "collab training queued from user activity");
            }
            Err(error) => warn!(%error, "collab training enqueue failed"),
        }
    }
}

struct EnqueueGuard<'a>(&'a AtomicBool);

impl Drop for EnqueueGuard<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

fn validate_payload(payload: CollabTrainPayload) -> AppResult<()> {
    if payload.min_count == Some(0) {
        return Err(AppError::bad_request("minCount must be greater than zero"));
    }
    if payload
        .min_count
        .is_some_and(|min_count| min_count > COLLAB_MAX_MIN_COUNT)
    {
        return Err(AppError::bad_request(format!(
            "minCount must not exceed {COLLAB_MAX_MIN_COUNT}"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_zero_training_parameters() {
        assert!(validate_payload(CollabTrainPayload { min_count: Some(0) }).is_err());
        assert!(
            validate_payload(CollabTrainPayload {
                min_count: Some(COLLAB_MAX_MIN_COUNT + 1),
            })
            .is_err()
        );
        assert!(validate_payload(CollabTrainPayload { min_count: None }).is_ok());
    }
}
