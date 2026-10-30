use backend_contracts::pipeline::STORAGE_TRACK_UPLOADED;
use chrono::NaiveDateTime;

use super::IndexingHandler;
use crate::queue::{JobError, JobResult};

const REPAIR_BATCH: i64 = 50;
const REPAIR_LOOKBACK_SECONDS: i64 = 2 * 24 * 60 * 60;
const REPAIR_GRACE_SECONDS: i64 = 30 * 60;

impl IndexingHandler {
    pub(super) async fn repair_lost_uploads(&self) -> JobResult {
        let settled = self
            .bus
            .storage_events_settled()
            .await
            .map_err(JobError::retryable)?;
        if !settled {
            tracing::debug!("storage events are still arriving; lost uploads are checked later");
            return Ok(());
        }
        let lost = sqlx::query_file!(
            "queries/indexing/storage/lost_uploads.sql",
            REPAIR_BATCH,
            REPAIR_LOOKBACK_SECONDS,
            REPAIR_GRACE_SECONDS
        )
        .fetch_all(&self.pool)
        .await
        .map_err(JobError::retryable)?;
        for upload in lost {
            self.bus
                .publish_dedup(
                    STORAGE_TRACK_UPLOADED,
                    &self.stored_upload(&upload.sc_track_id)?,
                    &repair_message_id(&upload.sc_track_id, upload.uploaded_at),
                )
                .await
                .map_err(JobError::retryable)?;
            sqlx::query_file!(
                "queries/indexing/storage/record_upload_repair.sql",
                upload.sc_track_id,
                upload.uploaded_at
            )
            .execute(&self.pool)
            .await
            .map_err(JobError::retryable)?;
            tracing::warn!(
                track = %upload.sc_track_id,
                uploaded_at = %upload.uploaded_at,
                "storage upload had no storage event; announced it again"
            );
        }
        Ok(())
    }
}

fn repair_message_id(sc_track_id: &str, uploaded_at: NaiveDateTime) -> String {
    format!(
        "storage-repair:{sc_track_id}:{}",
        uploaded_at.and_utc().timestamp_micros()
    )
}
