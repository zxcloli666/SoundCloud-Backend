use super::{JobRepository, validate_dedup_key};
use crate::queue::model::{NewJob, QueueError};
use sqlx::{PgConnection, Postgres, Transaction};

impl JobRepository {
    pub async fn enqueue(&self, job: &NewJob) -> Result<(), QueueError> {
        validate_dedup_key(job.dedup_key.as_deref())?;
        let mut connection = self.pool.acquire().await?;
        enqueue_on(&mut connection, job).await
    }

    pub async fn enqueue_in(
        &self,
        transaction: &mut Transaction<'_, Postgres>,
        job: &NewJob,
    ) -> Result<(), QueueError> {
        validate_dedup_key(job.dedup_key.as_deref())?;
        enqueue_on(transaction, job).await
    }

    pub async fn enqueue_if_absent(&self, job: &NewJob) -> Result<(), QueueError> {
        validate_dedup_key(job.dedup_key.as_deref())?;
        let mut connection = self.pool.acquire().await?;
        enqueue_if_absent_on(&mut connection, job).await
    }

    pub async fn enqueue_in_if_absent(
        &self,
        transaction: &mut Transaction<'_, Postgres>,
        job: &NewJob,
    ) -> Result<(), QueueError> {
        validate_dedup_key(job.dedup_key.as_deref())?;
        enqueue_if_absent_on(transaction, job).await
    }
}

async fn enqueue_if_absent_on(
    connection: &mut PgConnection,
    job: &NewJob,
) -> Result<(), QueueError> {
    sqlx::query_file!(
        "queries/queue/enqueue_if_absent.sql",
        job.id,
        job.kind.as_str(),
        job.kind.lane().as_str(),
        job.dedup_key.as_deref(),
        &job.payload,
        job.priority,
        job.max_attempts,
        job.available_at
    )
    .execute(connection)
    .await?;
    Ok(())
}

async fn enqueue_on(connection: &mut PgConnection, job: &NewJob) -> Result<(), QueueError> {
    sqlx::query_file!(
        "queries/queue/enqueue.sql",
        job.id,
        job.kind.as_str(),
        job.kind.lane().as_str(),
        job.dedup_key.as_deref(),
        &job.payload,
        job.priority,
        job.max_attempts,
        job.available_at
    )
    .execute(connection)
    .await?;
    Ok(())
}
