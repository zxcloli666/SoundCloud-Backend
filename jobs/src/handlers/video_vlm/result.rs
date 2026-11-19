use anyhow::{bail, ensure};
use backend_contracts::pipeline::{VideoClip, VideoLabel, VideoPart, VideoVlmResult};
use backend_contracts::reasons::{FailureClass, WorkerReason, WorkerStatus};
use backend_contracts::worker_contract::WorkerLane;
use serde_json::{Value, json};
use sqlx::PgPool;
use tracing::{debug, info, warn};

use super::VideoVlm;
use super::queue::{self, Held};
use crate::handlers::video_find::final_key;
use crate::objects::Blobs;
use crate::queue::{JobError, JobResult};

#[derive(Debug, PartialEq)]
pub(super) struct Answer {
    pub(super) label: VideoLabel,
    pub(super) model: String,
    pub(super) scores: Value,
    pub(super) duration_ms: Option<i32>,
    pub(super) file: Option<(VideoClip, VideoPart)>,
}

#[derive(Debug, PartialEq)]
pub(super) enum Verdict {
    Labelled(Box<Answer>),
    Drop(WorkerReason),
    Again(WorkerReason),
    Gone,
}

impl<B: Blobs + 'static> VideoVlm<B> {
    pub async fn finish(&self, result: VideoVlmResult) -> JobResult {
        let attempt = i32::try_from(result.attempt)
            .map_err(|_| JobError::permanent(anyhow::anyhow!("video attempt is out of range")))?;
        let track = result.sc_track_id.as_str();
        match judge(&result).map_err(JobError::permanent)? {
            Verdict::Labelled(answer) => self.labelled(track, attempt, &result, &answer).await,
            Verdict::Drop(reason) => self.dropped(track, attempt, reason).await,
            Verdict::Again(reason) => self.again(track, attempt, reason).await,
            Verdict::Gone => self.gone(track, attempt).await,
        }
    }

    async fn labelled(
        &self,
        track: &str,
        attempt: i32,
        result: &VideoVlmResult,
        answer: &Answer,
    ) -> JobResult {
        let file = match &answer.file {
            Some((clip, part)) => {
                let joined = self
                    .parts
                    .add(track, result.attempt, clip, part)
                    .map_err(JobError::permanent)?;
                let Some(joined) = joined else {
                    return Ok(());
                };
                Some(joined)
            }
            None => None,
        };
        let Some(held) = queue::held(&self.pool, track, attempt).await? else {
            debug!(track, attempt, "stale VLM result ignored");
            return Ok(());
        };
        match (answer.label, file) {
            (VideoLabel::Clip, Some(file)) => {
                self.keep_clip(track, attempt, &held, answer, file).await?
            }
            (VideoLabel::Loop, _) => self.keep_loop(track, attempt, &held, answer).await?,
            (VideoLabel::Static, _) => {
                let wait = self.pacing.static_recheck;
                let why = Some(WorkerReason::StaticVideo.as_str());
                queue::settle(&self.pool, track, attempt, "static", wait, why).await?;
                info!(
                    track,
                    attempt,
                    model = answer.model,
                    "VLM found a static video"
                );
            }
            (VideoLabel::Clip, None) => {
                return Err(JobError::permanent(anyhow::anyhow!(
                    "VLM clip carries no file"
                )));
            }
        }
        self.drop_temp(&held.s3_key).await;
        Ok(())
    }

    async fn keep_clip(
        &self,
        track: &str,
        attempt: i32,
        held: &Held,
        answer: &Answer,
        file: Vec<u8>,
    ) -> JobResult {
        let key = final_key(track, attempt);
        self.blobs
            .put_bytes(&key, file)
            .await
            .map_err(JobError::retryable)?;
        let duration_ms = answer.duration_ms.unwrap_or(held.duration_ms);
        let stored = store(
            &self.pool,
            track,
            attempt,
            Stored {
                kind: VideoLabel::Clip,
                offset_ms: held.offset_ms,
                duration_ms,
                key: &key,
            },
            held,
            answer,
        )
        .await?;
        if !stored {
            let _ = self.blobs.delete(&key).await;
        }
        Ok(())
    }

    async fn keep_loop(
        &self,
        track: &str,
        attempt: i32,
        held: &Held,
        answer: &Answer,
    ) -> JobResult {
        let key = final_key(track, attempt);
        self.blobs
            .copy(&held.s3_key, &key)
            .await
            .map_err(JobError::retryable)?;
        let stored = store(
            &self.pool,
            track,
            attempt,
            Stored {
                kind: VideoLabel::Loop,
                offset_ms: 0,
                duration_ms: held.duration_ms,
                key: &key,
            },
            held,
            answer,
        )
        .await?;
        if !stored {
            let _ = self.blobs.delete(&key).await;
        }
        Ok(())
    }

    async fn dropped(&self, track: &str, attempt: i32, reason: WorkerReason) -> JobResult {
        let Some(held) = queue::held(&self.pool, track, attempt).await? else {
            debug!(track, attempt, "stale VLM result ignored");
            return Ok(());
        };
        let wait = self.pacing.none_recheck;
        queue::settle(
            &self.pool,
            track,
            attempt,
            "none",
            wait,
            Some(reason.as_str()),
        )
        .await?;
        warn!(
            track,
            attempt,
            reason = reason.as_str(),
            "VLM check failed for good"
        );
        self.drop_temp(&held.s3_key).await;
        Ok(())
    }

    async fn again(&self, track: &str, attempt: i32, reason: WorkerReason) -> JobResult {
        let Some(held) = queue::held(&self.pool, track, attempt).await? else {
            debug!(track, attempt, "stale VLM result ignored");
            return Ok(());
        };
        let stage = queue::again(&self.pool, track, attempt, reason.as_str(), &self.pacing).await?;
        warn!(
            track,
            attempt,
            reason = reason.as_str(),
            stage,
            "VLM check failed; it will be sent again"
        );
        if stage.as_deref() == Some("none") {
            self.drop_temp(&held.s3_key).await;
        }
        Ok(())
    }

    async fn gone(&self, track: &str, attempt: i32) -> JobResult {
        let Some(held) = queue::held(&self.pool, track, attempt).await? else {
            debug!(track, attempt, "stale VLM result ignored");
            return Ok(());
        };
        let reason = WorkerReason::InputsGone.as_str();
        let stage = queue::rebuild(&self.pool, track, attempt, reason, &self.pacing).await?;
        info!(
            track,
            attempt, stage, "held video is gone; the search starts over"
        );
        self.drop_temp(&held.s3_key).await;
        Ok(())
    }

    async fn drop_temp(&self, key: &str) {
        if let Err(error) = self.blobs.delete(key).await {
            warn!(key, %error, "a video temp file was not deleted; the sweep removes it");
        }
    }
}

struct Stored<'a> {
    kind: VideoLabel,
    offset_ms: i32,
    duration_ms: i32,
    key: &'a str,
}

async fn store(
    pool: &PgPool,
    track: &str,
    attempt: i32,
    stored: Stored<'_>,
    held: &Held,
    answer: &Answer,
) -> JobResult<bool> {
    let mut transaction = pool.begin().await.map_err(JobError::retryable)?;
    let settled = sqlx::query_file!(
        "queries/video_vlm/settle.sql",
        track,
        attempt,
        "found",
        0.0_f64,
        None::<&str>
    )
    .fetch_optional(&mut *transaction)
    .await
    .map_err(JobError::retryable)?;
    if settled.is_none() {
        debug!(track, attempt, "stale VLM result ignored");
        return Ok(false);
    }
    sqlx::query_file!(
        "queries/video_find/store_video.sql",
        track,
        stored.kind.as_str(),
        held.source.as_str(),
        held.source_ref,
        stored.key,
        stored.offset_ms,
        stored.kind == VideoLabel::Loop,
        stored.duration_ms,
        verify(held, answer)
    )
    .execute(&mut *transaction)
    .await
    .map_err(JobError::retryable)?;
    transaction.commit().await.map_err(JobError::retryable)?;
    info!(
        track,
        attempt,
        kind = stored.kind.as_str(),
        model = answer.model,
        "video stored after the VLM check"
    );
    Ok(true)
}

fn verify(held: &Held, answer: &Answer) -> Value {
    let mut verify = held.scores.clone();
    if let Value::Object(map) = &mut verify {
        map.insert(
            "vlm".to_owned(),
            json!({
                "label": answer.label.as_str(),
                "model": answer.model,
                "scores": answer.scores,
            }),
        );
    }
    verify
}

pub(super) fn judge(result: &VideoVlmResult) -> anyhow::Result<Verdict> {
    ensure!(
        catalog_ingest::normalize_sc_track_id(&result.sc_track_id).as_deref()
            == Some(result.sc_track_id.as_str()),
        "VLM result has a non-canonical track id"
    );
    match (result.status, result.reason) {
        (WorkerStatus::Ok, None) => Ok(Verdict::Labelled(Box::new(answer(result)?))),
        (WorkerStatus::Ok, Some(reason)) => {
            bail!("VLM ok result carries reason {}", reason.as_str())
        }
        (status, None) => bail!("VLM {} result has no reason", status.as_str()),
        (status, Some(reason)) => {
            ensure!(
                reason.status() == status && reason.is_published_on(WorkerLane::VideoVlm),
                "VLM result carries reason {} its lane never publishes as {}",
                reason.as_str(),
                status.as_str()
            );
            Ok(match reason.failure_class() {
                _ if reason == WorkerReason::InputsGone => Verdict::Gone,
                Some(FailureClass::Transient | FailureClass::Reopenable) => Verdict::Again(reason),
                _ => Verdict::Drop(reason),
            })
        }
    }
}

fn answer(result: &VideoVlmResult) -> anyhow::Result<Answer> {
    let (Some(label), Some(model)) = (result.label, result.model.clone()) else {
        bail!("VLM ok result lacks its label or model");
    };
    let file = match (&result.clip, &result.part) {
        (Some(clip), Some(part)) => Some((clip.clone(), part.clone())),
        (None, None) => None,
        _ => bail!("VLM result carries half a file"),
    };
    ensure!(
        file.is_some() == (label == VideoLabel::Clip),
        "VLM {} result {} a file",
        label.as_str(),
        if label == VideoLabel::Clip {
            "lacks"
        } else {
            "carries"
        }
    );
    if label == VideoLabel::Clip {
        ensure!(
            result.duration_ms.is_some(),
            "VLM clip result lacks its duration"
        );
    }
    Ok(Answer {
        label,
        model,
        scores: serde_json::to_value(result.scores.clone().unwrap_or_default())?,
        duration_ms: result.duration_ms,
        file,
    })
}
