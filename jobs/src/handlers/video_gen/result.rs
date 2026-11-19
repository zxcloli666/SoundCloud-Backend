use std::time::Duration;

use backend_contracts::reasons::{WorkerReason, WorkerStatus};
use backend_contracts::video_gen::VideoGenResult;
use backend_contracts::worker_contract::WorkerLane;
use serde_json::json;
use tracing::{debug, info};

use super::director::Director;
use super::lanes::Lanes;
use super::parts::Delivered;
use super::state::{self, Gen, Save, Stage};
use super::{Store, VideoGen};
use crate::handlers::video_accounts::Notify;
use crate::queue::{JobError, JobResult};

impl<D: Director, S: Store, L: Lanes, N: Notify> VideoGen<D, S, L, N> {
    pub async fn finish(&self, lane: WorkerLane, result: VideoGenResult) -> JobResult {
        let out_of_range =
            || JobError::permanent(anyhow::anyhow!("generation result is out of range"));
        let attempt = i32::try_from(result.attempt).map_err(|_| out_of_range())?;
        let step = i32::try_from(result.step).map_err(|_| out_of_range())?;
        let track = result.sc_track_id.as_str();
        let Some(job) = state::sent(&self.pool, track, attempt, step).await? else {
            debug!(track, attempt, step, "stale generation result ignored");
            return Ok(());
        };
        if job.stage != stage_of(lane) {
            debug!(
                track,
                step,
                stage = job.stage.as_str(),
                "generation result for another stage ignored"
            );
            return Ok(());
        }
        if result.status != WorkerStatus::Ok {
            return self.failed(job, lane, &result).await;
        }
        let (Some(clip), Some(files), Some(part)) = (&result.clip, &result.files, &result.part)
        else {
            return self.fault(job, "an ok answer came without files").await;
        };
        let blob = self
            .parts
            .add((track, result.attempt, result.step), clip, part)
            .map_err(JobError::permanent)?;
        let Some(blob) = blob else {
            return Ok(());
        };
        let delivered = Delivered::split(&blob, files).map_err(JobError::permanent)?;
        self.delivered(job, lane, delivered).await
    }

    async fn failed(&self, mut job: Gen, lane: WorkerLane, result: &VideoGenResult) -> JobResult {
        let reason = result.reason.unwrap_or(WorkerReason::InternalError);
        if reason == WorkerReason::InputsGone {
            return self.gone(job, lane).await;
        }
        if reason.is_reopenable_on(lane) {
            return self.resend(&job).await;
        }
        let detail = result.detail.as_deref().unwrap_or_default();
        job.reject(lane.as_str(), reason.as_str(), detail);
        self.fault(
            job,
            &format!("{} {}: {detail}", lane.as_str(), reason.as_str()),
        )
        .await
    }

    async fn gone(&self, mut job: Gen, lane: WorkerLane) -> JobResult {
        info!(track = %job.track, lane = lane.as_str(), "generation inputs are gone");
        match lane {
            WorkerLane::VideoKeyframe => {
                job.end(Stage::Failed, "the cover is gone");
                return self.close(job).await;
            }
            WorkerLane::VideoShot => {
                job.state.frame = None;
                job.stage = Stage::Keyframe;
            }
            _ => {
                job.state.accepted.clear();
                job.state.frames.clear();
                job.state.frame = None;
                job.shot = 0;
                job.redo = 0;
                job.stage = Stage::Keyframe;
            }
        }
        self.resend(&job).await
    }

    async fn resend(&self, job: &Gen) -> JobResult {
        let again = Save {
            sent: false,
            wait: Duration::ZERO,
        };
        state::save(&self.pool, job, job.step, again).await?;
        Ok(())
    }

    async fn delivered(&self, mut job: Gen, lane: WorkerLane, mut files: Delivered) -> JobResult {
        let mut made = Vec::new();
        match lane {
            WorkerLane::VideoKeyframe => {
                let key = self
                    .keep(
                        &job,
                        "frame.png",
                        files.take("frame.png").map_err(JobError::permanent)?,
                    )
                    .await?;
                job.state.frame = Some(key.clone());
                made.push(key);
                job.stage = Stage::KeyframeReview;
            }
            WorkerLane::VideoShot => {
                let key = self
                    .keep(
                        &job,
                        "shot.mp4",
                        files.take("shot.mp4").map_err(JobError::permanent)?,
                    )
                    .await?;
                job.state.shot = Some(key.clone());
                made.push(key);
                let mut stills = Vec::new();
                for (name, data) in files.stills() {
                    stills.push(self.keep(&job, &name, data).await?);
                }
                if stills.is_empty() {
                    self.drop_temps(&made).await;
                    return self.fault(job, "a shot came without stills").await;
                }
                made.extend(stills.iter().cloned());
                job.state.stills = stills;
                job.stage = Stage::ShotReview;
            }
            _ => {
                return self
                    .store_video(job, files.take("video.mp4").map_err(JobError::permanent)?)
                    .await;
            }
        }
        job.state.temps.extend(made.iter().cloned());
        job.faults = 0;
        if !state::save(
            &self.pool,
            &job,
            job.step,
            Save {
                sent: false,
                wait: Duration::ZERO,
            },
        )
        .await?
        {
            self.drop_temps(&made).await;
        }
        Ok(())
    }

    async fn keep(&self, job: &Gen, name: &str, data: Vec<u8>) -> JobResult<String> {
        let key = job.temp_key(name);
        self.store
            .put_bytes(&key, data)
            .await
            .map_err(JobError::retryable)?;
        Ok(key)
    }

    async fn store_video(&self, mut job: Gen, video: Vec<u8>) -> JobResult {
        let plan = job
            .plan
            .clone()
            .ok_or_else(|| JobError::permanent(anyhow::anyhow!("a video without a plan")))?;
        let key = job.final_key();
        self.store
            .put_bytes(&key, video)
            .await
            .map_err(JobError::retryable)?;
        let offset = plan.shots[0].start_ms;
        let duration: i32 = plan
            .shots
            .iter()
            .map(|shot| shot.end_ms - shot.start_ms)
            .sum();
        let shots: Vec<_> = plan
            .shots
            .iter()
            .zip(&job.state.accepted)
            .map(|(shot, accepted)| {
                json!({
                    "start_ms": shot.start_ms,
                    "end_ms": shot.end_ms,
                    "trim_start_ms": accepted.trim_start_ms,
                })
            })
            .collect();
        let verify = json!({
            "mood": plan.mood,
            "character": plan.character,
            "shots": shots,
            "rejections": job.rejections,
        });
        sqlx::query_file!(
            "queries/video_gen/store_video.sql",
            job.track,
            key,
            offset,
            duration,
            verify
        )
        .execute(&self.pool)
        .await
        .map_err(JobError::retryable)?;
        info!(track = %job.track, attempt = job.attempt, "generated video stored");
        job.stage = Stage::Done;
        job.reason = None;
        self.close(job).await
    }
}

fn stage_of(lane: WorkerLane) -> Stage {
    match lane {
        WorkerLane::VideoKeyframe => Stage::Keyframe,
        WorkerLane::VideoShot => Stage::Shot,
        _ => Stage::Assemble,
    }
}
