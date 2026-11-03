use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use super::Kind;

#[derive(Debug, Serialize)]
pub struct EnqueuedJob {
    pub id: Uuid,
    pub kind: String,
    pub lane: String,
    pub dedup_key: String,
    pub payload: Value,
    pub priority: i16,
    pub max_attempts: i16,
    pub raise_priority: bool,
    pub star_candidates: Vec<String>,
    pub star_bonus: i16,
}

#[derive(Debug, Serialize)]
pub struct ObserveClaim {
    pub playlist_urn: String,
    pub at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub struct JobEnqueue {
    pub jobs: Vec<EnqueuedJob>,
    pub claim: Option<ObserveClaim>,
}

#[derive(Debug, PartialEq, Eq, Deserialize)]
pub struct Enqueued {
    pub claimed: bool,
    pub inserted: i64,
}

impl Kind for JobEnqueue {
    const NAME: &'static str = "job.enqueue";
    type Output = Option<Enqueued>;
}

#[derive(Debug, Serialize)]
pub struct LyricsWake {
    pub sc_track_id: String,
    pub at: DateTime<Utc>,
}

#[derive(Debug, PartialEq, Eq, Deserialize)]
pub struct LyricsWakeState {
    pub status: String,
    pub next_run_at: DateTime<Utc>,
    pub failure_streak: i32,
    pub claim_job_id: Option<Uuid>,
}

impl Kind for LyricsWake {
    const NAME: &'static str = "lyrics.wake";
    type Output = Option<LyricsWakeState>;
}

#[derive(Debug, Serialize)]
#[serde(tag = "mode", rename_all = "lowercase")]
pub enum ColdRefreshClear {
    Audience {
        subject_urn: String,
        subject_id: String,
        relations: Vec<String>,
    },
    Comments {
        sc_track_id: String,
    },
}

impl Kind for ColdRefreshClear {
    const NAME: &'static str = "cold_refresh.clear";
    type Output = i64;
}

#[derive(Debug, Serialize)]
pub struct AppTokenReject {
    pub oauth_app_id: Uuid,
    pub generation: Uuid,
    pub at: DateTime<Utc>,
}

impl Kind for AppTokenReject {
    const NAME: &'static str = "app_token.reject";
    type Output = bool;
}
