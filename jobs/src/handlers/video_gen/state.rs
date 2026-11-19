use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use sqlx::PgPool;

use super::director::Plan;
use crate::queue::{JobError, JobResult};

pub const LANE_STAGES: [&str; 3] = ["keyframe", "shot", "assemble"];
pub const DIRECTOR_STAGES: [&str; 3] = ["plan", "keyframe_review", "shot_review"];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    Plan,
    Keyframe,
    KeyframeReview,
    Shot,
    ShotReview,
    Assemble,
    Done,
    Rejected,
    Failed,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct State {
    #[serde(default)]
    pub cover: String,
    #[serde(default)]
    pub edit: Option<String>,
    #[serde(default)]
    pub frame: Option<String>,
    #[serde(default)]
    pub shot: Option<String>,
    #[serde(default)]
    pub stills: Vec<String>,
    #[serde(default)]
    pub frames: Vec<String>,
    #[serde(default)]
    pub accepted: Vec<Accepted>,
    #[serde(default)]
    pub temps: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct Accepted {
    pub key: String,
    pub trim_start_ms: i32,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct Rejection {
    pub shot: i32,
    pub look: String,
    pub reason: String,
    #[serde(default)]
    pub edit: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Gen {
    pub track: String,
    pub stage: Stage,
    pub attempt: i32,
    pub step: i32,
    pub shot: i32,
    pub redo: i32,
    pub faults: i32,
    pub plan: Option<Plan>,
    pub state: State,
    pub rejections: Vec<Rejection>,
    pub reason: Option<String>,
}

#[derive(sqlx::FromRow)]
pub struct Row {
    pub sc_track_id: String,
    pub stage: String,
    pub attempt: i32,
    pub step: i32,
    pub shot: i32,
    pub redo: i32,
    pub faults: i32,
    pub plan: Option<Value>,
    pub state: Value,
    pub rejections: Value,
}

pub struct Save {
    pub sent: bool,
    pub wait: Duration,
}

impl Stage {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Plan => "plan",
            Self::Keyframe => "keyframe",
            Self::KeyframeReview => "keyframe_review",
            Self::Shot => "shot",
            Self::ShotReview => "shot_review",
            Self::Assemble => "assemble",
            Self::Done => "done",
            Self::Rejected => "rejected",
            Self::Failed => "failed",
        }
    }

    fn parse(text: &str) -> Option<Self> {
        [
            Self::Plan,
            Self::Keyframe,
            Self::KeyframeReview,
            Self::Shot,
            Self::ShotReview,
            Self::Assemble,
            Self::Done,
            Self::Rejected,
            Self::Failed,
        ]
        .into_iter()
        .find(|stage| stage.as_str() == text)
    }
}

impl Gen {
    pub fn from_row(row: Row) -> JobResult<Self> {
        let stage = Stage::parse(&row.stage)
            .ok_or_else(|| JobError::permanent(anyhow::anyhow!("unknown stage {}", row.stage)))?;
        Ok(Self {
            track: row.sc_track_id,
            stage,
            attempt: row.attempt,
            step: row.step,
            shot: row.shot,
            redo: row.redo,
            faults: row.faults,
            plan: row
                .plan
                .map(serde_json::from_value)
                .transpose()
                .map_err(JobError::permanent)?,
            state: serde_json::from_value(row.state).map_err(JobError::permanent)?,
            rejections: serde_json::from_value(row.rejections).map_err(JobError::permanent)?,
            reason: None,
        })
    }

    pub fn temp_key(&self, name: &str) -> String {
        format!(
            "video-tmp/gen/{}-{}-{}-{name}",
            self.track, self.attempt, self.step
        )
    }

    pub fn final_key(&self) -> String {
        format!("video/{}-gen-{}.mp4", self.track, self.attempt)
    }

    pub fn message_id(&self, prefix: &str) -> String {
        format!("{prefix}:{}:{}:{}", self.track, self.attempt, self.step)
    }

    pub fn seed(&self) -> u32 {
        let digest = Sha256::digest(self.message_id("seed").as_bytes());
        u32::from_be_bytes([digest[0], digest[1], digest[2], digest[3]])
    }

    pub fn reject(&mut self, look: &str, reason: &str, edit: &str) {
        self.rejections.push(Rejection {
            shot: self.shot,
            look: look.to_owned(),
            reason: reason.to_owned(),
            edit: edit.to_owned(),
        });
    }

    pub fn end(&mut self, stage: Stage, reason: &str) {
        self.stage = stage;
        self.reason = Some(reason.to_owned());
    }
}

pub async fn enqueue(pool: &PgPool, limit: i64) -> JobResult<u64> {
    let done = sqlx::query_file!("queries/video_gen/enqueue.sql", limit)
        .execute(pool)
        .await
        .map_err(JobError::retryable)?;
    Ok(done.rows_affected())
}

pub async fn busy(pool: &PgPool) -> JobResult<(i64, i64)> {
    let row = sqlx::query_file!("queries/video_gen/busy.sql")
        .fetch_one(pool)
        .await
        .map_err(JobError::retryable)?;
    Ok((row.sent, row.active))
}

pub async fn requeue_unanswered(pool: &PgPool, timeout: Duration) -> JobResult {
    sqlx::query_file!("queries/video_gen/unanswered.sql", timeout.as_secs_f64())
        .execute(pool)
        .await
        .map_err(JobError::retryable)?;
    Ok(())
}

pub async fn claim(
    pool: &PgPool,
    limit: i64,
    stages: &[String],
    lease: Duration,
) -> JobResult<Vec<Gen>> {
    let rows = sqlx::query_file_as!(
        Row,
        "queries/video_gen/claim.sql",
        limit,
        stages,
        lease.as_secs_f64()
    )
    .fetch_all(pool)
    .await
    .map_err(JobError::retryable)?;
    rows.into_iter().map(Gen::from_row).collect()
}

pub async fn sent(pool: &PgPool, track: &str, attempt: i32, step: i32) -> JobResult<Option<Gen>> {
    let row = sqlx::query_file_as!(Row, "queries/video_gen/sent.sql", track, attempt, step)
        .fetch_optional(pool)
        .await
        .map_err(JobError::retryable)?;
    row.map(Gen::from_row).transpose()
}

pub async fn save(pool: &PgPool, job: &Gen, expected_step: i32, save: Save) -> JobResult<bool> {
    let plan = job
        .plan
        .as_ref()
        .map(serde_json::to_value)
        .transpose()
        .map_err(JobError::permanent)?;
    let state = serde_json::to_value(&job.state).map_err(JobError::permanent)?;
    let rejections = serde_json::to_value(&job.rejections).map_err(JobError::permanent)?;
    let done = sqlx::query_file!(
        "queries/video_gen/save.sql",
        job.track,
        job.attempt,
        expected_step,
        job.stage.as_str(),
        job.step,
        job.shot,
        job.redo,
        job.faults,
        plan,
        state,
        rejections,
        job.reason.as_deref(),
        save.sent,
        save.wait.as_secs_f64()
    )
    .execute(pool)
    .await
    .map_err(JobError::retryable)?;
    Ok(done.rows_affected() == 1)
}
