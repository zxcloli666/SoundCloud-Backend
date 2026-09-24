use std::sync::Arc;
use std::time::Duration;

use tokio::time::{Instant, MissedTickBehavior};

use crate::config::QueueConfig;
use crate::handlers::JobHandlers;

use super::model::{JobError, JobResult, LeasedJob, QueueError};
use super::repository::{Completion, JobRepository};

enum Execution {
    Finished(JobResult),
    LostLease,
}

pub(super) async fn process_job(
    repository: JobRepository,
    handlers: Arc<JobHandlers>,
    config: QueueConfig,
    job: LeasedJob,
) -> Result<(), QueueError> {
    tracing::info!(job_id = %job.id, kind = %job.kind, attempt = job.attempts, "background job started");

    let started = Instant::now();
    let execution = execute_with_heartbeat(&repository, &handlers, &config, &job).await?;
    let result = match execution {
        Execution::Finished(result) => result,
        Execution::LostLease => {
            tracing::warn!(job_id = %job.id, kind = %job.kind, "background job lease was lost");
            return Ok(());
        }
    };

    crate::metrics::record_execution(
        job.kind.as_str(),
        match &result {
            Ok(()) => crate::metrics::Outcome::Ok,
            Err(error) if cancelled_by_timeout(error) => crate::metrics::Outcome::Timeout,
            Err(error) if error.is_retryable() => crate::metrics::Outcome::Retryable,
            Err(_) => crate::metrics::Outcome::Terminal,
        },
        started.elapsed(),
    );

    match result {
        Ok(()) => {
            let completion = repository.complete(&job).await?;
            log_completion(&job, completion, true);
        }
        Err(JobError::Postponed { delay, error }) => {
            let completion = repository.postpone(&job, &error.to_string(), delay).await?;
            log_completion(&job, completion, false);
        }
        Err(error) => {
            let retryable = error.is_retryable();
            let completion = repository.fail(&job, &error.to_string(), retryable).await?;
            log_completion(&job, completion, false);
        }
    }

    Ok(())
}

fn cancelled_by_timeout(error: &JobError) -> bool {
    let source = match error {
        JobError::Retryable(error) | JobError::Permanent(error) => error,
        JobError::Postponed { error, .. } => error,
    };
    source.chain().any(|cause| {
        cause
            .downcast_ref::<sqlx::Error>()
            .and_then(|error| error.as_database_error())
            .and_then(|error| error.code())
            .is_some_and(|code| code == "57014")
    })
}

async fn execute_with_heartbeat(
    repository: &JobRepository,
    handlers: &JobHandlers,
    config: &QueueConfig,
    job: &LeasedJob,
) -> Result<Execution, QueueError> {
    drive(
        handlers.handle(job),
        config.job_timeout,
        config.heartbeat_interval,
        || repository.heartbeat(job, config.lease_duration),
    )
    .await
}

async fn drive<W, H, B>(
    work: W,
    job_timeout: Duration,
    heartbeat_interval: Duration,
    mut renew_lease: H,
) -> Result<Execution, QueueError>
where
    W: Future<Output = JobResult>,
    H: FnMut() -> B,
    B: Future<Output = Result<bool, QueueError>>,
{
    let deadline = tokio::time::sleep(job_timeout);
    let mut heartbeat =
        tokio::time::interval_at(Instant::now() + heartbeat_interval, heartbeat_interval);
    heartbeat.set_missed_tick_behavior(MissedTickBehavior::Delay);
    tokio::pin!(work);
    tokio::pin!(deadline);

    loop {
        tokio::select! {
            biased;
            result = &mut work => return Ok(Execution::Finished(result)),
            _ = &mut deadline => {
                let error = anyhow::anyhow!(
                    "job exceeded its {:.3}s execution limit",
                    job_timeout.as_secs_f64()
                );
                return Ok(Execution::Finished(Err(JobError::retryable(error))));
            }
            _ = heartbeat.tick() => {
                if !renew_lease().await? {
                    return Ok(Execution::LostLease);
                }
            }
        }
    }
}

fn log_completion(job: &LeasedJob, completion: Completion, succeeded: bool) {
    match completion {
        Completion::Completed => tracing::info!(
            job_id = %job.id,
            kind = %job.kind,
            succeeded,
            "background job finished"
        ),
        Completion::Superseded => tracing::info!(
            job_id = %job.id,
            kind = %job.kind,
            "background job was superseded"
        ),
        Completion::LostLease => tracing::warn!(
            job_id = %job.id,
            kind = %job.kind,
            "background job terminal write lost its lease"
        ),
    }
}

#[cfg(test)]
mod timeout_tests {
    use super::*;

    async fn cancelled(pool: &sqlx::PgPool) -> sqlx::Error {
        let mut connection = pool.acquire().await.expect("connection");
        sqlx::query("SET statement_timeout = '50ms'")
            .execute(&mut *connection)
            .await
            .expect("timeout applies to this connection");
        sqlx::query("SELECT pg_sleep(1)")
            .execute(&mut *connection)
            .await
            .expect_err("the statement must be cancelled")
    }

    #[sqlx::test(migrations = false)]
    async fn a_statement_cancelled_by_its_timeout_is_reported_as_a_timeout(
        pool: sqlx::PgPool,
    ) -> anyhow::Result<()> {
        let error = JobError::retryable(cancelled(&pool).await);

        assert!(
            cancelled_by_timeout(&error),
            "a cancelled bulk statement must be visible as a timeout, not a generic retry"
        );
        Ok(())
    }

    #[test]
    fn an_ordinary_failure_is_not_a_timeout() {
        let ordinary = JobError::retryable(anyhow::anyhow!("connection reset"));
        let permanent = JobError::permanent(anyhow::anyhow!("invalid payload"));

        assert!(!cancelled_by_timeout(&ordinary));
        assert!(!cancelled_by_timeout(&permanent));
    }
}

#[cfg(test)]
mod deadline_tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    const JOB_TIMEOUT: Duration = Duration::from_secs(900);
    const HEARTBEAT: Duration = Duration::from_secs(30);

    #[tokio::test(start_paused = true)]
    async fn a_read_that_never_answers_is_cut_off_by_the_job_deadline() {
        let beats = AtomicUsize::new(0);
        let execution = drive(
            std::future::pending::<JobResult>(),
            JOB_TIMEOUT,
            HEARTBEAT,
            || {
                beats.fetch_add(1, Ordering::SeqCst);
                async { Ok(true) }
            },
        )
        .await
        .expect("driving a job never fails on its own");

        match execution {
            Execution::Finished(Err(error)) => {
                assert!(
                    error.is_retryable(),
                    "a job cut off by its deadline must come back, not die"
                );
                assert!(
                    error.to_string().contains("execution limit"),
                    "the reason must name the deadline, saw {error}"
                );
            }
            _ => panic!("a job that never answers must be cut off by its own deadline"),
        }
        assert_eq!(
            beats.load(Ordering::SeqCst),
            29,
            "the lease must be renewed for the whole run, otherwise the job is picked up twice"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_slow_job_keeps_its_lease_and_still_finishes() {
        let beats = AtomicUsize::new(0);
        let work = async {
            tokio::time::sleep(HEARTBEAT * 3 + Duration::from_secs(1)).await;
            Ok(())
        };
        let execution = drive(work, JOB_TIMEOUT, HEARTBEAT, || {
            beats.fetch_add(1, Ordering::SeqCst);
            async { Ok(true) }
        })
        .await
        .expect("driving a job never fails on its own");

        assert!(matches!(execution, Execution::Finished(Ok(()))));
        assert_eq!(beats.load(Ordering::SeqCst), 3);
    }

    #[tokio::test(start_paused = true)]
    async fn a_job_whose_lease_was_taken_away_stops_instead_of_finishing() {
        let beats = AtomicUsize::new(0);
        let execution = drive(
            std::future::pending::<JobResult>(),
            JOB_TIMEOUT,
            HEARTBEAT,
            || {
                let seen = beats.fetch_add(1, Ordering::SeqCst) + 1;
                async move { Ok(seen < 2) }
            },
        )
        .await
        .expect("driving a job never fails on its own");

        assert!(matches!(execution, Execution::LostLease));
        assert_eq!(beats.load(Ordering::SeqCst), 2);
    }
}
