use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::pipeline::{
    HTTP_URL_PATTERN, MAX_DETAIL_CHARS, MAX_URL_CHARS, Producer, SC_TRACK_ID_PATTERN, VideoClip,
    VideoPart,
};
use crate::reasons::{WorkerReason, WorkerStatus};

pub const MAX_VIDEO_PROMPT_CHARS: u32 = 4000;
pub const MAX_VIDEO_SHOTS: u32 = 12;
pub const MAX_VIDEO_FILES: u32 = 8;
pub const MAX_VIDEO_FILE_NAME_CHARS: u32 = 32;
pub const MAX_VIDEO_SIDE: u32 = 1280;
pub const MAX_SHOT_FRAMES: u32 = 161;
pub const MAX_SHOT_STILLS: u32 = 8;
pub const MAX_VIDEO_GEN_MS: i32 = 120_000;

#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Eq, Serialize)]
pub struct VideoKeyframeRequest {
    #[schemars(pattern(SC_TRACK_ID_PATTERN))]
    pub sc_track_id: String,
    #[schemars(range(min = 1))]
    pub attempt: i64,
    #[schemars(range(min = 1))]
    pub step: i64,
    #[schemars(pattern(HTTP_URL_PATTERN), length(max = MAX_URL_CHARS))]
    pub cover_url: String,
    #[schemars(length(min = 1, max = MAX_VIDEO_PROMPT_CHARS))]
    pub prompt: String,
    #[schemars(length(max = MAX_VIDEO_PROMPT_CHARS))]
    pub negative_prompt: String,
    #[schemars(range(min = 64, max = MAX_VIDEO_SIDE))]
    pub width: u32,
    #[schemars(range(min = 64, max = MAX_VIDEO_SIDE))]
    pub height: u32,
    pub seed: u32,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Eq, Serialize)]
pub struct VideoShotRequest {
    #[schemars(pattern(SC_TRACK_ID_PATTERN))]
    pub sc_track_id: String,
    #[schemars(range(min = 1))]
    pub attempt: i64,
    #[schemars(range(min = 1))]
    pub step: i64,
    #[schemars(pattern(HTTP_URL_PATTERN), length(max = MAX_URL_CHARS))]
    pub frame_url: String,
    #[schemars(length(min = 1, max = MAX_VIDEO_PROMPT_CHARS))]
    pub prompt: String,
    #[schemars(length(max = MAX_VIDEO_PROMPT_CHARS))]
    pub negative_prompt: String,
    #[schemars(range(min = 64, max = MAX_VIDEO_SIDE))]
    pub width: u32,
    #[schemars(range(min = 64, max = MAX_VIDEO_SIDE))]
    pub height: u32,
    #[schemars(range(min = 5, max = MAX_SHOT_FRAMES))]
    pub frames: u32,
    #[schemars(range(min = 1, max = 60))]
    pub fps: u32,
    #[schemars(range(min = 1, max = MAX_SHOT_STILLS))]
    pub stills: u32,
    pub seed: u32,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Eq, Serialize)]
pub struct VideoAssembleShot {
    #[schemars(pattern(HTTP_URL_PATTERN), length(max = MAX_URL_CHARS))]
    pub url: String,
    #[schemars(range(min = 0))]
    pub trim_start_ms: i32,
    #[schemars(range(min = 1, max = MAX_VIDEO_GEN_MS))]
    pub duration_ms: i32,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Eq, Serialize)]
pub struct VideoAssembleRequest {
    #[schemars(pattern(SC_TRACK_ID_PATTERN))]
    pub sc_track_id: String,
    #[schemars(range(min = 1))]
    pub attempt: i64,
    #[schemars(range(min = 1))]
    pub step: i64,
    #[schemars(pattern(HTTP_URL_PATTERN), length(max = MAX_URL_CHARS))]
    pub audio_url: String,
    #[schemars(range(min = 0))]
    pub audio_offset_ms: i32,
    #[schemars(length(min = 1, max = MAX_VIDEO_SHOTS))]
    pub shots: Vec<VideoAssembleShot>,
    #[schemars(range(min = 64, max = MAX_VIDEO_SIDE))]
    pub width: u32,
    #[schemars(range(min = 64, max = MAX_VIDEO_SIDE))]
    pub height: u32,
    #[schemars(range(min = 1, max = 60))]
    pub fps: u32,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Eq, Serialize)]
pub struct VideoFileSlice {
    #[schemars(length(min = 1, max = MAX_VIDEO_FILE_NAME_CHARS))]
    pub name: String,
    #[schemars(range(min = 0))]
    pub offset: u64,
    #[schemars(range(min = 1))]
    pub bytes: u64,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
pub struct VideoGenResult {
    #[schemars(pattern(SC_TRACK_ID_PATTERN))]
    pub sc_track_id: String,
    #[schemars(range(min = 1))]
    pub attempt: i64,
    #[schemars(range(min = 1))]
    pub step: i64,
    pub status: WorkerStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "WorkerReason")]
    pub reason: Option<WorkerReason>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "String", length(max = MAX_DETAIL_CHARS))]
    pub detail: Option<String>,
    pub producer: Producer,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "VideoClip")]
    pub clip: Option<VideoClip>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "Vec<VideoFileSlice>", length(min = 1, max = MAX_VIDEO_FILES))]
    pub files: Option<Vec<VideoFileSlice>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "VideoPart")]
    pub part: Option<VideoPart>,
}
