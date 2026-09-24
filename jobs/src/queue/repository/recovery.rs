use super::JobRepository;
use crate::queue::model::QueueError;

impl JobRepository {
    pub async fn recover_exhausted(&self, limit: usize) -> Result<u64, QueueError> {
        if limit == 0 {
            return Ok(0);
        }

        let limit = i64::try_from(limit).map_err(|_| QueueError::BatchTooLarge)?;
        let result = sqlx::query_file!("queries/queue/recover_exhausted.sql", limit)
            .execute(&self.pool)
            .await?;

        Ok(result.rows_affected())
    }
}
