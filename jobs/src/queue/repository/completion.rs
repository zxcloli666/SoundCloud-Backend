use std::time::Duration;

use super::{Completion, JobRepository, duration_milliseconds, truncate};
use crate::queue::backoff;
use crate::queue::model::{LeasedJob, QueueError};

const MAX_ERROR_LENGTH: usize = 2_000;

impl JobRepository {
    pub async fn postpone(
        &self,
        job: &LeasedJob,
        error: &str,
        delay: Duration,
    ) -> Result<Completion, QueueError> {
        let error = truncate(error, MAX_ERROR_LENGTH);
        let delay_milliseconds =
            duration_milliseconds(delay.clamp(Duration::from_secs(1), Duration::from_secs(86400)))?;
        let updated = sqlx::query_file!(
            "queries/queue/postpone.sql",
            job.id,
            job.lease_id,
            job.generation,
            delay_milliseconds,
            error
        )
        .execute(&self.pool)
        .await?
        .rows_affected();
        if updated == 1 {
            return Ok(Completion::Completed);
        }
        self.release_superseded(job).await
    }

    pub async fn complete(&self, job: &LeasedJob) -> Result<Completion, QueueError> {
        let deleted = sqlx::query_file!(
            "queries/queue/complete.sql",
            job.id,
            job.lease_id,
            job.generation
        )
        .execute(&self.pool)
        .await?
        .rows_affected();

        if deleted == 1 {
            return Ok(Completion::Completed);
        }

        self.release_superseded(job).await
    }

    pub async fn fail(
        &self,
        job: &LeasedJob,
        error: &str,
        retryable: bool,
    ) -> Result<Completion, QueueError> {
        if self.has_released_newer_generation(job).await? {
            return Ok(Completion::Superseded);
        }

        let error = truncate(error, MAX_ERROR_LENGTH);
        if !retryable || job.attempts >= i32::from(job.max_attempts) {
            return self.dead_letter(job, error.as_str()).await;
        }

        let delay = backoff::retry_delay(job.id, job.attempts);
        let delay_milliseconds = duration_milliseconds(delay)?;
        let updated = sqlx::query_file!(
            "queries/queue/retry.sql",
            job.id,
            job.lease_id,
            job.generation,
            delay_milliseconds,
            error.as_str()
        )
        .execute(&self.pool)
        .await?
        .rows_affected();

        if updated == 1 {
            return Ok(Completion::Completed);
        }

        self.release_superseded(job).await
    }

    pub async fn heartbeat(
        &self,
        job: &LeasedJob,
        lease_duration: Duration,
    ) -> Result<bool, QueueError> {
        let lease_milliseconds = duration_milliseconds(lease_duration)?;
        let updated = sqlx::query_file!(
            "queries/queue/heartbeat.sql",
            job.id,
            job.lease_id,
            job.generation,
            lease_milliseconds
        )
        .execute(&self.pool)
        .await?
        .rows_affected();

        Ok(updated == 1)
    }

    async fn has_released_newer_generation(&self, job: &LeasedJob) -> Result<bool, QueueError> {
        let released = sqlx::query_file!(
            "queries/queue/release_superseded.sql",
            job.id,
            job.lease_id,
            job.generation
        )
        .execute(&self.pool)
        .await?
        .rows_affected();

        Ok(released == 1)
    }

    async fn release_superseded(&self, job: &LeasedJob) -> Result<Completion, QueueError> {
        let released = self.has_released_newer_generation(job).await?;
        Ok(if released {
            Completion::Superseded
        } else {
            Completion::LostLease
        })
    }

    async fn dead_letter(&self, job: &LeasedJob, error: &str) -> Result<Completion, QueueError> {
        let moved = sqlx::query_file!(
            "queries/queue/dead_letter.sql",
            job.id,
            job.lease_id,
            job.generation,
            error
        )
        .execute(&self.pool)
        .await?
        .rows_affected();

        if moved == 1 {
            return Ok(Completion::Completed);
        }

        self.release_superseded(job).await
    }
}
