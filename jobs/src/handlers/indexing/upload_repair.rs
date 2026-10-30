use std::time::{Duration, Instant};

use backend_contracts::pipeline::STORAGE_TRACK_UPLOADED;
use chrono::NaiveDateTime;
use serde_json::json;

use super::IndexingHandler;
use crate::bus::StorageEventsProgress;
use crate::metrics::{self, RepairPauseReason};
use crate::queue::{JobError, JobResult};

const REPAIR_BATCH: i64 = 50;
const REPAIR_LOOKBACK_SECONDS: i64 = 2 * 24 * 60 * 60;
const REPAIR_GRACE_SECONDS: i64 = 30 * 60;
const PAUSE_WARNING_EVERY: Duration = Duration::from_secs(60 * 60);

pub(super) struct RepairPause {
    since: Instant,
    warned_at: Option<Instant>,
}

impl RepairPause {
    pub(super) fn starting_now() -> Self {
        Self {
            since: Instant::now(),
            warned_at: None,
        }
    }
}

impl IndexingHandler {
    pub(super) async fn repair_lost_uploads(&self) -> JobResult {
        let progress = match self.bus.storage_events_progress().await {
            Ok(progress) => progress,
            Err(error) => {
                self.hold_repairs(RepairPauseReason::StorageEventsUnreadable, &error);
                return Err(JobError::retryable(error));
            }
        };
        if let Some(reason) = pause_reason(&progress) {
            self.hold_repairs(
                reason,
                &format!(
                    "{} leaf sources behind, {} uploads undelivered",
                    progress.sources_behind, progress.undelivered
                ),
            );
            return Ok(());
        }
        self.resume_repairs();

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
            let event = json!({
                "sc_track_id": upload.sc_track_id,
                "storage_url": self.storage_audio_url(&upload.sc_track_id)?,
                "quality": upload.stored_quality,
            });
            self.bus
                .publish_dedup(
                    STORAGE_TRACK_UPLOADED,
                    &event,
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
                quality = upload.stored_quality.as_deref().unwrap_or("unknown"),
                "storage upload had no storage event; announced it again"
            );
        }
        Ok(())
    }

    fn hold_repairs(&self, reason: RepairPauseReason, detail: &dyn std::fmt::Display) {
        let Ok(mut pause) = self.repair_pause.lock() else {
            return;
        };
        let now = Instant::now();
        let held = now.duration_since(pause.since);
        metrics::record_storage_repair_paused(Some((reason, held)));
        if warning_due(held, pause.warned_at.map(|at| now.duration_since(at))) {
            pause.warned_at = Some(now);
            tracing::warn!(
                reason = reason.as_str(),
                paused_seconds = held.as_secs(),
                "repairs of uploads without a storage event are paused: {detail}"
            );
        }
    }

    fn resume_repairs(&self) {
        if let Ok(mut pause) = self.repair_pause.lock() {
            *pause = RepairPause::starting_now();
        }
        metrics::record_storage_repair_paused(None);
    }
}

fn pause_reason(progress: &StorageEventsProgress) -> Option<RepairPauseReason> {
    if progress.sources_behind > 0 {
        Some(RepairPauseReason::LeafSourceBehind)
    } else if progress.undelivered > 0 {
        Some(RepairPauseReason::UploadsUndelivered)
    } else {
        None
    }
}

fn warning_due(held: Duration, since_warning: Option<Duration>) -> bool {
    held >= PAUSE_WARNING_EVERY && since_warning.is_none_or(|ago| ago >= PAUSE_WARNING_EVERY)
}

fn repair_message_id(sc_track_id: &str, uploaded_at: NaiveDateTime) -> String {
    format!(
        "storage-repair:{sc_track_id}:{}",
        uploaded_at.and_utc().timestamp_micros()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const MINUTE: Duration = Duration::from_secs(60);

    #[test]
    fn a_pause_warns_after_an_hour_and_then_once_an_hour() {
        assert!(!warning_due(59 * MINUTE, None));
        assert!(warning_due(60 * MINUTE, None));
        assert!(!warning_due(90 * MINUTE, Some(30 * MINUTE)));
        assert!(warning_due(120 * MINUTE, Some(60 * MINUTE)));
    }

    #[test]
    fn a_leaf_behind_outranks_undelivered_uploads() {
        let progress = |sources_behind, undelivered| StorageEventsProgress {
            sources_behind,
            undelivered,
        };

        assert_eq!(
            pause_reason(&progress(1, 5)),
            Some(RepairPauseReason::LeafSourceBehind)
        );
        assert_eq!(
            pause_reason(&progress(0, 5)),
            Some(RepairPauseReason::UploadsUndelivered)
        );
        assert_eq!(pause_reason(&progress(0, 0)), None);
    }
}
