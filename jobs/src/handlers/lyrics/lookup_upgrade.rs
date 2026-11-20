use super::LyricsLookupHandler;
use crate::queue::{JobError, JobResult};

impl LyricsLookupHandler {
    pub(super) async fn enqueue_synced_upgrades(&self) -> JobResult {
        if self.config.upgrade_batch == 0 {
            return Ok(());
        }
        sqlx::query_file!(
            "queries/lyrics/lookup_enqueue_upgrades.sql",
            self.config.upgrade_batch,
            self.config.upgrade_age_days
        )
        .execute(&self.pool)
        .await
        .map_err(JobError::retryable)?;
        Ok(())
    }
}
