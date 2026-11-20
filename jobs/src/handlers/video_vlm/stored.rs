use backend_contracts::reasons::WorkerReason;
use serde_json::{Value, json};
use tracing::{debug, info, warn};

use super::VideoVlm;
use super::haiku::Vision;
use super::queue::{self, Held, Known};
use crate::handlers::video_accounts::Notify;
use crate::handlers::video_gen::Nodes;
use crate::objects::Blobs;
use crate::queue::{JobError, JobResult};

impl<B: Blobs + 'static, L: Nodes, V: Vision, N: Notify> VideoVlm<B, L, V, N> {
    pub(super) async fn verify_stored(&self) -> JobResult {
        if self.config.verify_stored <= 0 || queue::waiting(&self.pool).await? > 0 {
            return Ok(());
        }
        if !self.haiku_open().await? {
            return Ok(());
        }
        let sent = sqlx::query_file!(
            "queries/video_vlm/stored_enqueue.sql",
            self.config.verify_stored
        )
        .fetch_all(&self.pool)
        .await
        .map_err(JobError::retryable)?;
        if !sent.is_empty() {
            info!(count = sent.len(), "stored videos sent to the VLM check");
        }
        Ok(())
    }

    pub(super) async fn keep_stored(
        &self,
        track: &str,
        attempt: i32,
        held: &Held,
        known: &Known,
    ) -> JobResult {
        let verdict = json!({
            "label": known.label.as_str(),
            "model": known.model,
            "scores": known.scores,
        });
        if self.close_stored(track, attempt, held, verdict).await? {
            info!(
                track,
                label = known.label.as_str(),
                model = known.model,
                "stored video passed the VLM check"
            );
        }
        Ok(())
    }

    pub(super) async fn fail_stored(
        &self,
        track: &str,
        attempt: i32,
        held: &Held,
        reason: WorkerReason,
    ) -> JobResult {
        let verdict = json!({"failed": reason.as_str()});
        if self.close_stored(track, attempt, held, verdict).await? {
            warn!(
                track,
                reason = reason.as_str(),
                "stored video VLM check failed for good; the video stays"
            );
        }
        Ok(())
    }

    pub(super) async fn mark_stored(
        &self,
        track: &str,
        held: &Held,
        reason: WorkerReason,
    ) -> JobResult {
        sqlx::query_file!(
            "queries/video_vlm/stored_mark.sql",
            track,
            held.s3_key,
            json!({"failed": reason.as_str()})
        )
        .execute(&self.pool)
        .await
        .map_err(JobError::retryable)?;
        Ok(())
    }

    pub(super) async fn drop_stored(
        &self,
        track: &str,
        attempt: i32,
        held: &Held,
        model: &str,
    ) -> JobResult {
        let mut transaction = self.pool.begin().await.map_err(JobError::retryable)?;
        let dropped = sqlx::query_file!("queries/video_vlm/stored_drop.sql", track, held.s3_key)
            .fetch_optional(&mut *transaction)
            .await
            .map_err(JobError::retryable)?;
        let settled = sqlx::query_file!(
            "queries/video_vlm/settle.sql",
            track,
            attempt,
            "static",
            self.pacing.static_recheck.as_secs_f64(),
            Some(WorkerReason::StaticVideo.as_str()),
            self.pacing.research_fresh.as_secs_f64(),
            self.pacing.research_old.as_secs_f64(),
            self.pacing.research_fresh_age.as_secs_f64()
        )
        .fetch_optional(&mut *transaction)
        .await
        .map_err(JobError::retryable)?;
        if settled.is_none() {
            debug!(track, attempt, "stale VLM result ignored");
            return Ok(());
        }
        transaction.commit().await.map_err(JobError::retryable)?;
        if let Some(dropped) = dropped {
            info!(
                track,
                kind = dropped.kind,
                model,
                "VLM found a stored video static; it is deleted"
            );
            self.drop_temp(&held.s3_key).await;
        }
        Ok(())
    }

    pub(super) async fn gone_stored(&self, track: &str, attempt: i32, held: &Held) -> JobResult {
        let mut transaction = self.pool.begin().await.map_err(JobError::retryable)?;
        sqlx::query_file!("queries/video_vlm/stored_drop.sql", track, held.s3_key)
            .fetch_optional(&mut *transaction)
            .await
            .map_err(JobError::retryable)?;
        let rebuilt = sqlx::query_file!(
            "queries/video_vlm/rebuild.sql",
            track,
            attempt,
            self.pacing.retry_base.as_secs_f64(),
            i32::try_from(self.pacing.max_attempts).unwrap_or(i32::MAX),
            WorkerReason::InputsGone.as_str(),
            self.pacing.none_recheck.as_secs_f64()
        )
        .fetch_optional(&mut *transaction)
        .await
        .map_err(JobError::retryable)?;
        let Some(rebuilt) = rebuilt else {
            debug!(track, attempt, "stale VLM result ignored");
            return Ok(());
        };
        transaction.commit().await.map_err(JobError::retryable)?;
        warn!(
            track,
            key = held.s3_key,
            stage = rebuilt.stage,
            "stored video file is gone; its row is deleted"
        );
        Ok(())
    }

    async fn close_stored(
        &self,
        track: &str,
        attempt: i32,
        held: &Held,
        verdict: Value,
    ) -> JobResult<bool> {
        let closed = sqlx::query_file!(
            "queries/video_vlm/stored_close.sql",
            track,
            attempt,
            held.s3_key,
            verdict
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(JobError::retryable)?;
        if closed.is_none() {
            debug!(track, attempt, "stale VLM result ignored");
        }
        Ok(closed.is_some())
    }
}
