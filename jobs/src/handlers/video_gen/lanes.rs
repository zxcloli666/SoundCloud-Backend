use std::future::Future;
use std::time::{Duration, Instant};

use backend_contracts::pipeline::{VIDEO_ASSEMBLE, VIDEO_KEYFRAME, VIDEO_SHOT};
use backend_contracts::video_gen::{
    VideoAssembleRequest, VideoAssembleShot, VideoKeyframeRequest, VideoShotRequest,
};
use backend_contracts::worker_contract::WorkerLane;
use serde_json::Value;
use url::Url;

use super::prompts;
use super::state::Gen;
use crate::bus::Bus;
use crate::config::VideoGenConfig;
use crate::handlers::indexing::canonical_storage_url;
use crate::health::worker_status::WorkerStatusBoard;
use crate::queue::{JobError, JobResult};

pub const STAGE_LANES: [(&str, WorkerLane); 3] = [
    ("keyframe", WorkerLane::VideoKeyframe),
    ("shot", WorkerLane::VideoShot),
    ("assemble", WorkerLane::VideoAssemble),
];

pub trait Lanes: Send + Sync {
    fn publish(
        &self,
        subject: &'static str,
        payload: Value,
        message_id: String,
    ) -> impl Future<Output = anyhow::Result<()>> + Send;
}

pub trait Nodes: Lanes {
    fn serving(&self, lane: WorkerLane) -> bool;
}

impl Lanes for Bus {
    async fn publish(
        &self,
        subject: &'static str,
        payload: Value,
        message_id: String,
    ) -> anyhow::Result<()> {
        self.publish_dedup(subject, &payload, &message_id).await
    }
}

pub struct Fleet {
    bus: Bus,
    workers: WorkerStatusBoard,
    seen: Duration,
}

impl Fleet {
    pub fn new(bus: Bus, workers: WorkerStatusBoard, seen: Duration) -> Self {
        Self { bus, workers, seen }
    }
}

impl Lanes for Fleet {
    async fn publish(
        &self,
        subject: &'static str,
        payload: Value,
        message_id: String,
    ) -> anyhow::Result<()> {
        self.bus.publish_dedup(subject, &payload, &message_id).await
    }
}

impl Nodes for Fleet {
    fn serving(&self, lane: WorkerLane) -> bool {
        self.workers.served_within(lane, self.seen, Instant::now())
    }
}

pub struct Outgoing {
    pub subject: &'static str,
    pub payload: Value,
    pub message_id: String,
}

pub fn keyframe(job: &Gen, config: &VideoGenConfig) -> JobResult<Outgoing> {
    let plan = planned(job)?;
    let shot = job.shot as usize;
    let request = VideoKeyframeRequest {
        sc_track_id: job.track.clone(),
        attempt: i64::from(job.attempt),
        step: i64::from(job.step),
        cover_url: job.state.cover.clone(),
        prompt: prompts::keyframe_prompt(plan, shot, job.state.edit.as_deref()),
        negative_prompt: prompts::negative_prompt(plan),
        width: config.side,
        height: config.side,
        seed: job.seed(),
    };
    outgoing(VIDEO_KEYFRAME, &request, job.message_id("video_keyframe"))
}

pub fn shot(job: &Gen, config: &VideoGenConfig, storage: &Url) -> JobResult<Outgoing> {
    let plan = planned(job)?;
    let frame = job
        .state
        .frame
        .as_deref()
        .ok_or_else(|| JobError::permanent(anyhow::anyhow!("a shot without a keyframe")))?;
    let request = VideoShotRequest {
        sc_track_id: job.track.clone(),
        attempt: i64::from(job.attempt),
        step: i64::from(job.step),
        frame_url: joined(storage, frame)?,
        prompt: prompts::motion_prompt(plan, job.shot as usize, job.state.edit.as_deref()),
        negative_prompt: prompts::negative_prompt(plan),
        width: config.side,
        height: config.side,
        frames: config.frames,
        fps: config.fps,
        stills: config.stills,
        seed: job.seed(),
    };
    outgoing(VIDEO_SHOT, &request, job.message_id("video_shot"))
}

pub fn assemble(job: &Gen, config: &VideoGenConfig, storage: &Url) -> JobResult<Outgoing> {
    let plan = planned(job)?;
    let shots = plan
        .shots
        .iter()
        .zip(&job.state.accepted)
        .map(|(planned, accepted)| {
            Ok(VideoAssembleShot {
                url: joined(storage, &accepted.key)?,
                trim_start_ms: accepted.trim_start_ms,
                duration_ms: planned.end_ms - planned.start_ms,
            })
        })
        .collect::<JobResult<Vec<_>>>()?;
    let request = VideoAssembleRequest {
        sc_track_id: job.track.clone(),
        attempt: i64::from(job.attempt),
        step: i64::from(job.step),
        audio_url: canonical_storage_url(storage, &job.track).map_err(JobError::permanent)?,
        audio_offset_ms: plan.shots[0].start_ms,
        shots,
        width: config.side,
        height: config.side,
        fps: config.fps,
    };
    outgoing(VIDEO_ASSEMBLE, &request, job.message_id("video_assemble"))
}

fn planned(job: &Gen) -> JobResult<&super::director::Plan> {
    job.plan
        .as_ref()
        .ok_or_else(|| JobError::permanent(anyhow::anyhow!("track {} has no plan", job.track)))
}

fn joined(storage: &Url, key: &str) -> JobResult<String> {
    Ok(storage.join(key).map_err(JobError::permanent)?.to_string())
}

fn outgoing(
    subject: &'static str,
    request: &impl serde::Serialize,
    message_id: String,
) -> JobResult<Outgoing> {
    Ok(Outgoing {
        subject,
        payload: serde_json::to_value(request).map_err(JobError::permanent)?,
        message_id,
    })
}
