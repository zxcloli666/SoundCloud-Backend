#[cfg(test)]
#[path = "search_terms_tests.rs"]
mod tests;

use sqlx::{Executor, PgPool};

use crate::queue::{JobError, JobResult};

pub struct SearchTermsHandler {
    pool: PgPool,
}

impl SearchTermsHandler {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub async fn refresh(&self) -> JobResult {
        let mut transaction = self.pool.begin().await.map_err(JobError::retryable)?;
        transaction
            .execute(sqlx::raw_sql(include_str!(
                "../../queries/search/refresh_terms.sql"
            )))
            .await
            .map_err(JobError::retryable)?;
        transaction.commit().await.map_err(JobError::retryable)?;
        Ok(())
    }
}
