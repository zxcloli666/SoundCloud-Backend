use std::sync::Arc;

use backend_contracts::JobKind;
use tokio::task::{JoinError, JoinSet};
use tokio_util::sync::CancellationToken;

use crate::config::{QueueConfig, QueueLaneConfig};
use crate::handlers::JobHandlers;

use super::ClaimOrder;
use super::execution::process_job;
use super::model::{LeasedJob, QueueError};
use super::repository::JobRepository;

const RECOVERY_INTERVAL: std::time::Duration = std::time::Duration::from_secs(60);
const RECOVERY_BATCH: usize = 100;

pub(crate) struct QueueWorker {
    repository: JobRepository,
    handlers: Arc<JobHandlers>,
    config: QueueConfig,
    lane: QueueLaneConfig,
    kinds: &'static [JobKind],
}

struct JobTaskExit {
    id: uuid::Uuid,
    kind: JobKind,
    result: Result<(), QueueError>,
}

impl QueueWorker {
    pub(crate) fn new(
        repository: JobRepository,
        handlers: Arc<JobHandlers>,
        config: QueueConfig,
        lane: QueueLaneConfig,
        kinds: &'static [JobKind],
    ) -> Self {
        Self {
            repository,
            handlers,
            config,
            lane,
            kinds,
        }
    }

    pub(crate) async fn run(self, cancellation: CancellationToken) -> anyhow::Result<()> {
        let mut tasks = JoinSet::new();
        let mut successful_claims = 0_u64;
        let mut empty_polls = 0_u32;

        'worker: loop {
            observe_ready(&mut tasks)?;

            if cancellation.is_cancelled() {
                break;
            }

            let limit = claim_limit(self.lane.concurrency, tasks.len(), self.lane.claim_batch);

            if limit == 0 {
                tokio::select! {
                    biased;
                    _ = cancellation.cancelled() => break 'worker,
                    task = tasks.join_next() => observe_optional(task)?,
                }
                continue;
            }

            let order = next_claim_order(successful_claims);
            let claimed = self
                .repository
                .claim(self.kinds, order, limit, self.config.lease_duration)
                .await;

            match claimed {
                Ok(jobs) if !jobs.is_empty() => {
                    successful_claims = successful_claims.wrapping_add(1);
                    empty_polls = 0;
                    for job in jobs {
                        spawn_job(
                            &mut tasks,
                            self.repository.clone(),
                            self.handlers.clone(),
                            self.config.clone(),
                            job,
                        );
                    }
                }
                Ok(_) => {
                    successful_claims = successful_claims.wrapping_add(1);
                    empty_polls = empty_polls.saturating_add(1);
                    let wait = super::backoff::idle_wait(empty_polls, self.config.poll_interval);
                    wait_for_poll(&mut tasks, wait, &cancellation).await?;
                }
                Err(error) => {
                    tracing::warn!(error = %error, "background job claim failed");
                    wait_for_poll(&mut tasks, self.config.poll_interval, &cancellation).await?;
                }
            }
        }

        while let Some(task) = tasks.join_next().await {
            observe_task(task)?;
        }
        Ok(())
    }
}

pub(crate) async fn recover_exhausted(
    repository: JobRepository,
    cancellation: CancellationToken,
) -> anyhow::Result<()> {
    let mut interval = tokio::time::interval(RECOVERY_INTERVAL);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            _ = cancellation.cancelled() => return Ok(()),
            _ = interval.tick() => recover_once(&repository).await,
        }
    }
}

fn spawn_job(
    tasks: &mut JoinSet<JobTaskExit>,
    repository: JobRepository,
    handlers: Arc<JobHandlers>,
    config: QueueConfig,
    job: LeasedJob,
) {
    let id = job.id;
    let kind = job.kind;
    tasks.spawn(async move {
        let result = process_job(repository, handlers, config, job).await;
        JobTaskExit { id, kind, result }
    });
}

fn claim_limit(concurrency: usize, running: usize, batch: usize) -> usize {
    concurrency.saturating_sub(running).min(batch)
}

fn next_claim_order(successful_claims: u64) -> ClaimOrder {
    if successful_claims.wrapping_add(1).is_multiple_of(32) {
        ClaimOrder::Oldest
    } else {
        ClaimOrder::Priority
    }
}

async fn recover_once(repository: &JobRepository) {
    match repository.recover_exhausted(RECOVERY_BATCH).await {
        Ok(0) => {}
        Ok(recovered) => {
            tracing::info!(recovered, "exhausted background jobs archived");
        }
        Err(error) => {
            tracing::warn!(error = %error, "exhausted background job recovery failed");
        }
    }
}

fn observe_ready(tasks: &mut JoinSet<JobTaskExit>) -> anyhow::Result<()> {
    while let Some(task) = tasks.try_join_next() {
        observe_task(task)?;
    }
    Ok(())
}

fn observe_optional(task: Option<Result<JobTaskExit, JoinError>>) -> anyhow::Result<()> {
    match task {
        Some(task) => observe_task(task),
        None => Ok(()),
    }
}

fn observe_task(task: Result<JobTaskExit, JoinError>) -> anyhow::Result<()> {
    match task {
        Ok(JobTaskExit {
            id,
            kind,
            result: Ok(()),
        }) => {
            tracing::debug!(job_id = %id, kind = %kind, "background job task released");
            Ok(())
        }
        Ok(JobTaskExit {
            id,
            kind,
            result: Err(error),
        }) => {
            tracing::error!(job_id = %id, kind = %kind, error = %error, "background job persistence failed");
            Ok(())
        }
        Err(error) => Err(anyhow::Error::new(error).context("background job task failed")),
    }
}

async fn wait_for_poll(
    tasks: &mut JoinSet<JobTaskExit>,
    poll_interval: std::time::Duration,
    cancellation: &CancellationToken,
) -> anyhow::Result<()> {
    tokio::select! {
        biased;
        _ = cancellation.cancelled() => Ok(()),
        task = tasks.join_next(), if !tasks.is_empty() => observe_optional(task),
        _ = tokio::time::sleep(poll_interval) => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claim_never_exceeds_free_concurrency() {
        assert_eq!(claim_limit(32, 31, 64), 1);
        assert_eq!(claim_limit(32, 10, 8), 8);
        assert_eq!(claim_limit(32, 32, 8), 0);
        assert_eq!(claim_limit(32, 40, 8), 0);
    }

    #[test]
    fn every_thirty_second_claim_serves_the_oldest_lane() {
        assert_eq!(next_claim_order(0), ClaimOrder::Priority);
        assert_eq!(next_claim_order(30), ClaimOrder::Priority);
        assert_eq!(next_claim_order(31), ClaimOrder::Oldest);
        assert_eq!(next_claim_order(63), ClaimOrder::Oldest);
    }
}
