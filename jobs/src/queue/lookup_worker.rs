use std::sync::Arc;

use backend_contracts::JobKind;
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;

use crate::config::QueueConfig;
use crate::handlers::{JobHandlers, LOOKUP_KINDS};

use super::lookup_lane::LookupLane;
use super::model::ClaimOrder;
use super::repository::JobRepository;
use super::worker::{
    JobTaskExit, RELEASE_EXPIRED_INTERVAL, observe_ready, observe_task, release_expired, spawn_job,
    wait_for_poll,
};

const RELEASE_BATCH: usize = 256;

pub(crate) struct LookupWorker {
    repository: JobRepository,
    handlers: Arc<JobHandlers>,
    config: QueueConfig,
    lane: Arc<LookupLane>,
}

impl LookupWorker {
    pub(crate) fn new(
        repository: JobRepository,
        handlers: Arc<JobHandlers>,
        config: QueueConfig,
        lane: Arc<LookupLane>,
    ) -> Self {
        Self {
            repository,
            handlers,
            config,
            lane,
        }
    }

    pub(crate) async fn run(self, cancellation: CancellationToken) -> anyhow::Result<()> {
        let mut lookups = JoinSet::new();
        let mut sweeps = JoinSet::new();
        let mut empty_polls = 0_u32;
        let mut next_release = tokio::time::Instant::now();

        loop {
            observe_ready(&mut lookups)?;
            observe_ready(&mut sweeps)?;

            if cancellation.is_cancelled() {
                break;
            }

            if tokio::time::Instant::now() >= next_release {
                next_release = tokio::time::Instant::now() + RELEASE_EXPIRED_INTERVAL;
                release_expired(&self.repository, LOOKUP_KINDS, RELEASE_BATCH).await;
                if sweeps.is_empty() {
                    self.start_sweep(&mut sweeps).await;
                }
            }

            let slots = self.lane.targeted_slots();
            if slots.is_empty() {
                wait_for_poll(&mut lookups, self.config.poll_interval, &cancellation).await?;
                continue;
            }

            match self
                .lane
                .claim_targeted(&self.repository, slots, self.config.lease_duration)
                .await
            {
                Ok(claimed) if !claimed.is_empty() => {
                    empty_polls = 0;
                    for (job, slot) in claimed {
                        spawn_job(
                            &mut lookups,
                            self.repository.clone(),
                            self.handlers.clone(),
                            self.config.clone(),
                            job,
                            slot,
                        );
                    }
                }
                Ok(_) => {
                    empty_polls = empty_polls.saturating_add(1);
                    let wait = super::backoff::idle_wait(empty_polls, self.config.poll_interval);
                    wait_for_poll(&mut lookups, wait, &cancellation).await?;
                }
                Err(error) => {
                    tracing::warn!(error = %error, "lyrics lookup claim failed");
                    wait_for_poll(&mut lookups, self.config.poll_interval, &cancellation).await?;
                }
            }
        }

        while let Some(task) = lookups.join_next().await {
            observe_task(task)?;
        }
        while let Some(task) = sweeps.join_next().await {
            observe_task(task)?;
        }
        Ok(())
    }

    async fn start_sweep(&self, sweeps: &mut JoinSet<JobTaskExit>) {
        let claimed = self
            .repository
            .claim(
                &[JobKind::LyricsLookupSweep],
                ClaimOrder::Kind(JobKind::LyricsLookupSweep),
                1,
                self.config.lease_duration,
            )
            .await;
        match claimed {
            Ok(jobs) => {
                for job in jobs {
                    spawn_job(
                        sweeps,
                        self.repository.clone(),
                        self.handlers.clone(),
                        self.config.clone(),
                        job,
                        (),
                    );
                }
            }
            Err(error) => tracing::warn!(error = %error, "lyrics lookup sweep claim failed"),
        }
    }
}
