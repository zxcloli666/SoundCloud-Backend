use anyhow::{bail, ensure};
use backend_contracts::pipeline::{VideoClip, VideoLabel, VideoPart, VideoVlmResult};
use backend_contracts::reasons::{FailureClass, WorkerReason, WorkerStatus};
use backend_contracts::video_gen::VideoFileSlice;
use backend_contracts::worker_contract::WorkerLane;
use serde_json::{Value, json};
use sqlx::PgPool;
use tracing::{debug, info, warn};

use super::VideoVlm;
use super::haiku::Vision;
use super::queue::{self, Held, HeldFile, Known};
use crate::handlers::video_accounts::Notify;
use crate::handlers::video_find::{Pacing, final_key};
use crate::handlers::video_gen::Nodes;
use crate::objects::Blobs;
use crate::queue::{JobError, JobResult};

#[derive(Debug, PartialEq)]
pub(super) struct Answer {
    pub(super) label: VideoLabel,
    pub(super) model: Option<String>,
    pub(super) scores: Value,
    pub(super) duration_ms: Option<i32>,
    pub(super) file: Option<(VideoClip, VideoPart)>,
}

#[derive(Debug, PartialEq)]
pub(super) struct Frames {
    pub(super) clip: VideoClip,
    pub(super) part: VideoPart,
    pub(super) files: Vec<VideoFileSlice>,
}

#[derive(Debug, PartialEq)]
pub(super) enum Verdict {
    Labelled(Box<Answer>),
    Answered(Box<Known>),
    Frames(Box<Frames>),
    Drop(WorkerReason),
    Again(WorkerReason),
    Fallback(WorkerReason),
    Gone,
}

impl<B: Blobs + 'static, L: Nodes, V: Vision, N: Notify> VideoVlm<B, L, V, N> {
    pub async fn finish(&self, lane: WorkerLane, result: VideoVlmResult) -> JobResult {
        let attempt = i32::try_from(result.attempt)
            .map_err(|_| JobError::permanent(anyhow::anyhow!("video attempt is out of range")))?;
        let track = result.sc_track_id.as_str();
        match judge(lane, &result).map_err(JobError::permanent)? {
            Verdict::Labelled(answer) => self.labelled(track, attempt, &result, &answer).await,
            Verdict::Answered(answer) => self.answered(track, attempt, &answer).await,
            Verdict::Frames(frames) => self.framed(track, attempt, &frames).await,
            Verdict::Drop(reason) => self.dropped(track, attempt, reason).await,
            Verdict::Again(reason) => self.again(track, attempt, reason).await,
            Verdict::Fallback(reason) => self.fallback(track, attempt, reason).await,
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
        let judged = judged_by(answer, &held).map_err(JobError::permanent)?;
        match (answer.label, file) {
            (VideoLabel::Clip | VideoLabel::Loop, Some(file)) => {
                self.keep_file(track, attempt, &held, &judged, file).await
            }
            (VideoLabel::Static, _) => {
                self.settle_static(track, attempt, &held, &judged.model)
                    .await
            }
            (_, None) => Err(JobError::permanent(anyhow::anyhow!(
                "VLM answer carries no file"
            ))),
        }
    }

    pub(super) async fn settle_static(
        &self,
        track: &str,
        attempt: i32,
        held: &Held,
        model: &str,
    ) -> JobResult {
        if held.file == HeldFile::Stored {
            return self.drop_stored(track, attempt, held, model).await;
        }
        if !held.next.is_empty() && queue::advance(&self.pool, track, attempt).await? {
            info!(
                track,
                attempt,
                model,
                left = held.next.len(),
                "VLM found the video static; the next candidate goes to the check"
            );
            self.drop_temp(&held.s3_key).await;
            return Ok(());
        }
        let wait = self.pacing.static_recheck;
        let why = Some(WorkerReason::StaticVideo.as_str());
        queue::settle(
            &self.pool,
            &self.pacing,
            track,
            attempt,
            "static",
            wait,
            why,
        )
        .await?;
        info!(track, attempt, model, "VLM found a static video");
        self.drop_temps(held).await;
        Ok(())
    }

    async fn keep_file(
        &self,
        track: &str,
        attempt: i32,
        held: &Held,
        answer: &Judged,
        file: Vec<u8>,
    ) -> JobResult {
        let key = final_key(track, attempt);
        self.blobs
            .put_bytes(&key, file)
            .await
            .map_err(JobError::retryable)?;
        self.keep(track, attempt, held, answer, &key).await
    }

    pub(super) async fn keep_cut(
        &self,
        track: &str,
        attempt: i32,
        held: &Held,
        known: &Known,
    ) -> JobResult {
        let key = final_key(track, attempt);
        self.blobs
            .copy(&held.s3_key, &key)
            .await
            .map_err(JobError::retryable)?;
        let answer = Judged {
            label: known.label,
            model: known.model.clone(),
            scores: Value::Object(known.scores.clone()),
            duration_ms: None,
        };
        self.keep(track, attempt, held, &answer, &key).await
    }

    async fn keep(
        &self,
        track: &str,
        attempt: i32,
        held: &Held,
        answer: &Judged,
        key: &str,
    ) -> JobResult {
        let duration_ms = answer.duration_ms.unwrap_or(held.duration_ms);
        let offset_ms = match answer.label {
            VideoLabel::Loop => 0,
            _ => held.offset_ms,
        };
        let stored = store(
            &self.pool,
            &self.pacing,
            track,
            attempt,
            Stored {
                kind: answer.label,
                offset_ms,
                duration_ms,
                key,
            },
            held,
            answer,
        )
        .await?;
        match stored {
            None => {
                let _ = self.blobs.delete(key).await;
            }
            Some(Some(replaced)) if replaced != key => self.drop_replaced(&replaced).await,
            Some(_) => {}
        }
        self.drop_temps(held).await;
        Ok(())
    }

    async fn dropped(&self, track: &str, attempt: i32, reason: WorkerReason) -> JobResult {
        let Some(held) = queue::held(&self.pool, track, attempt).await? else {
            debug!(track, attempt, "stale VLM result ignored");
            return Ok(());
        };
        if held.file == HeldFile::Stored {
            return self.fail_stored(track, attempt, &held, reason).await;
        }
        let wait = self.pacing.none_recheck;
        queue::settle(
            &self.pool,
            &self.pacing,
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
        self.drop_temps(&held).await;
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
        match (held.file, stage.as_deref()) {
            (_, None | Some("pending_vlm")) => {}
            (HeldFile::Stored, _) => self.mark_stored(track, &held, reason).await?,
            _ => self.drop_temps(&held).await,
        }
        Ok(())
    }

    async fn fallback(&self, track: &str, attempt: i32, reason: WorkerReason) -> JobResult {
        match queue::gpu_failed(&self.pool, track, attempt, reason.as_str()).await? {
            true => info!(
                track,
                attempt,
                reason = reason.as_str(),
                "GPU VLM check failed; it goes to the next path"
            ),
            false => debug!(track, attempt, "stale VLM result ignored"),
        }
        Ok(())
    }

    async fn gone(&self, track: &str, attempt: i32) -> JobResult {
        let Some(held) = queue::held(&self.pool, track, attempt).await? else {
            debug!(track, attempt, "stale VLM result ignored");
            return Ok(());
        };
        if held.file == HeldFile::Stored {
            return self.gone_stored(track, attempt, &held).await;
        }
        let reason = WorkerReason::InputsGone.as_str();
        let stage = queue::rebuild(&self.pool, track, attempt, reason, &self.pacing).await?;
        info!(
            track,
            attempt, stage, "held video is gone; the search starts over"
        );
        self.drop_temps(&held).await;
        Ok(())
    }

    async fn drop_temps(&self, held: &Held) {
        for key in held.temps() {
            self.drop_temp(key).await;
        }
    }

    pub(super) async fn drop_temp(&self, key: &str) {
        if let Err(error) = self.blobs.delete(key).await {
            warn!(key, %error, "a video temp file was not deleted; the sweep removes it");
        }
    }

    async fn drop_replaced(&self, key: &str) {
        if let Err(error) = self.blobs.delete(key).await {
            warn!(key, %error, "a replaced video file was not deleted");
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
    pacing: &Pacing,
    track: &str,
    attempt: i32,
    stored: Stored<'_>,
    held: &Held,
    answer: &Judged,
) -> JobResult<Option<Option<String>>> {
    let mut transaction = pool.begin().await.map_err(JobError::retryable)?;
    let settled = sqlx::query_file!(
        "queries/video_vlm/settle.sql",
        track,
        attempt,
        "found",
        0.0_f64,
        None::<&str>,
        pacing.research_fresh.as_secs_f64(),
        pacing.research_old.as_secs_f64(),
        pacing.research_fresh_age.as_secs_f64()
    )
    .fetch_optional(&mut *transaction)
    .await
    .map_err(JobError::retryable)?;
    if settled.is_none() {
        debug!(track, attempt, "stale VLM result ignored");
        return Ok(None);
    }
    let video = sqlx::query_file!(
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
    .fetch_one(&mut *transaction)
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
    Ok(Some(video.replaced))
}

struct Judged {
    label: VideoLabel,
    model: String,
    scores: Value,
    duration_ms: Option<i32>,
}

fn judged_by(answer: &Answer, held: &Held) -> anyhow::Result<Judged> {
    let (model, scores) = match (&answer.model, &held.haiku) {
        (Some(model), _) => (model.clone(), answer.scores.clone()),
        (None, Some(known)) if known.label == answer.label => {
            (known.model.clone(), Value::Object(known.scores.clone()))
        }
        _ => bail!("VLM {} answer names no model", answer.label.as_str()),
    };
    Ok(Judged {
        label: answer.label,
        model,
        scores,
        duration_ms: answer.duration_ms,
    })
}

fn verify(held: &Held, answer: &Judged) -> Value {
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

pub(super) fn judge(lane: WorkerLane, result: &VideoVlmResult) -> anyhow::Result<Verdict> {
    ensure!(
        catalog_ingest::normalize_sc_track_id(&result.sc_track_id).as_deref()
            == Some(result.sc_track_id.as_str()),
        "VLM result has a non-canonical track id"
    );
    match (result.status, result.reason) {
        (WorkerStatus::Ok, None) if result.label.is_none() => framed(result),
        (WorkerStatus::Ok, None) => labelled(result),
        (WorkerStatus::Ok, Some(reason)) => {
            bail!("VLM ok result carries reason {}", reason.as_str())
        }
        (status, None) => bail!("VLM {} result has no reason", status.as_str()),
        (status, Some(reason)) => {
            ensure!(
                reason.status() == status && reason.is_published_on(lane),
                "VLM result carries reason {} its lane never publishes as {}",
                reason.as_str(),
                status.as_str()
            );
            Ok(match reason.failure_class() {
                _ if reason == WorkerReason::InputsGone => Verdict::Gone,
                _ if reason == WorkerReason::PublicNodeTimeout => Verdict::Fallback(reason),
                Some(FailureClass::Transient | FailureClass::Reopenable) => Verdict::Again(reason),
                _ => Verdict::Drop(reason),
            })
        }
    }
}

fn framed(result: &VideoVlmResult) -> anyhow::Result<Verdict> {
    let (Some(clip), Some(part), Some(files)) = (&result.clip, &result.part, &result.files) else {
        bail!("VLM ok result has neither a label nor its frames");
    };
    let mut end = 0;
    for file in files {
        ensure!(
            file.offset == end && file.bytes > 0,
            "VLM frame {} does not follow the one before it",
            file.name
        );
        end += file.bytes;
    }
    ensure!(end == clip.bytes, "VLM frames do not cover the whole file");
    Ok(Verdict::Frames(Box::new(Frames {
        clip: clip.clone(),
        part: part.clone(),
        files: files.clone(),
    })))
}

fn labelled(result: &VideoVlmResult) -> anyhow::Result<Verdict> {
    let Some(label) = result.label else {
        bail!("VLM ok result lacks its label");
    };
    let model = result.model.clone();
    let file = match (&result.clip, &result.part) {
        (Some(clip), Some(part)) => Some((clip.clone(), part.clone())),
        (None, None) => None,
        _ => bail!("VLM result carries half a file"),
    };
    if label != VideoLabel::Static && file.is_none() {
        let Some(model) = model else {
            bail!("VLM {} answer names no model", label.as_str());
        };
        return Ok(Verdict::Answered(Box::new(Known {
            label,
            model,
            scores: result
                .scores
                .iter()
                .flatten()
                .map(|(name, score)| (name.clone(), json!(score)))
                .collect(),
        })));
    }
    ensure!(
        file.is_some() == (label != VideoLabel::Static),
        "VLM {} result {} a file",
        label.as_str(),
        if label == VideoLabel::Static {
            "carries"
        } else {
            "lacks"
        }
    );
    if label != VideoLabel::Static {
        ensure!(
            result.duration_ms.is_some(),
            "VLM {} result lacks its duration",
            label.as_str()
        );
    }
    Ok(Verdict::Labelled(Box::new(Answer {
        label,
        model,
        scores: serde_json::to_value(result.scores.clone().unwrap_or_default())?,
        duration_ms: result.duration_ms,
        file,
    })))
}
