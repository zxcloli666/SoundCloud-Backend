use std::future::Future;

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct Plan {
    pub character: Character,
    pub mood: String,
    pub bans: Vec<String>,
    pub shots: Vec<PlannedShot>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct Character {
    pub gender: String,
    pub hair: String,
    pub clothes: String,
    pub face_visible: bool,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct PlannedShot {
    pub start_ms: i32,
    pub end_ms: i32,
    pub line: String,
    pub keyframe: String,
    pub motion: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Brief {
    pub track: String,
    pub title: String,
    pub artist: String,
    pub duration_ms: i32,
    pub cover_url: String,
    pub lyrics: Option<String>,
    pub shots: usize,
    pub shot_ms: i32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Look {
    Keyframe,
    Shot,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Picture {
    pub name: String,
    pub media_type: &'static str,
    pub data: Vec<u8>,
}

#[derive(Clone, Debug)]
pub struct Review<'a> {
    pub look: Look,
    pub plan: &'a Plan,
    pub shot: usize,
    pub cover_url: &'a str,
    pub current: Vec<Picture>,
    pub earlier: Vec<Picture>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Verdict {
    Accept { trim_start_ms: i32 },
    Redo { reason: String, edit: String },
}

#[derive(Clone, Debug, PartialEq)]
pub struct Spend {
    pub call: &'static str,
    pub provider: String,
    pub model: String,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cache_read_tokens: i64,
    pub cache_write_tokens: i64,
    pub usd: f64,
}

#[derive(Clone, Debug, PartialEq, thiserror::Error)]
pub enum DirectorError {
    #[error("the director is unreachable: {0}")]
    Down(String),
    #[error("the director answered something unusable: {0}")]
    Invalid(String),
}

pub struct Answer<T> {
    pub result: Result<T, DirectorError>,
    pub spend: Vec<Spend>,
}

pub trait Director: Send + Sync {
    fn plan(&self, brief: &Brief) -> impl Future<Output = Answer<Plan>> + Send;

    fn review(&self, review: &Review<'_>) -> impl Future<Output = Answer<Verdict>> + Send;
}

pub fn checked(plan: Plan, brief: &Brief) -> Result<Plan, String> {
    if plan.shots.len() != brief.shots {
        return Err(format!(
            "{} shots, {} were asked for",
            plan.shots.len(),
            brief.shots
        ));
    }
    let mut end = plan.shots[0].start_ms;
    for (index, shot) in plan.shots.iter().enumerate() {
        let length = shot.end_ms - shot.start_ms;
        if shot.start_ms != end {
            return Err(format!(
                "shot {index} does not start where the last one ended"
            ));
        }
        if !(1_000..=brief.shot_ms).contains(&length) {
            return Err(format!(
                "shot {index} lasts {length} ms, not 1000..{}",
                brief.shot_ms
            ));
        }
        if shot.keyframe.trim().is_empty() || shot.motion.trim().is_empty() {
            return Err(format!("shot {index} has an empty prompt"));
        }
        end = shot.end_ms;
    }
    if plan.shots[0].start_ms < 0 || end > brief.duration_ms {
        return Err(format!(
            "shots run outside the track (0..{} ms)",
            brief.duration_ms
        ));
    }
    Ok(plan)
}
