use std::time::Duration;

use super::anthropic::{self, AnthropicLadder};
use super::env::{self, ConfigError};

const DEFAULT_LADDER: &str = "video";

#[derive(Clone, Debug)]
pub struct VideoGenConfig {
    pub in_flight: i64,
    pub tick: Duration,
    pub timeout: Duration,
    pub lease: Duration,
    pub month_usd: f64,
    pub recheck: Duration,
    pub retry_base: Duration,
    pub max_faults: i32,
    pub shots: usize,
    pub keyframe_tries: i32,
    pub shot_tries: i32,
    pub side: u32,
    pub fps: u32,
    pub frames: u32,
    pub stills: u32,
    pub director: Option<AnthropicLadder>,
}

impl VideoGenConfig {
    pub(super) fn from_env() -> Result<Self, ConfigError> {
        let in_flight: i64 = env::parse("VIDEO_GEN_IN_FLIGHT", "0")?;
        let director = match in_flight > 0 {
            true => anthropic::load("VIDEO_GEN_LADDER", Some(DEFAULT_LADDER))?,
            false => None,
        };
        let frames = bounded("VIDEO_GEN_FRAMES", 81, 5, 161)?;
        Ok(Self {
            in_flight,
            tick: seconds("VIDEO_GEN_TICK_SECONDS", 30)?,
            timeout: seconds("VIDEO_GEN_TIMEOUT_SECONDS", 3_600)?,
            lease: seconds("VIDEO_GEN_LEASE_SECONDS", 900)?,
            month_usd: anthropic::month_usd("VIDEO_GEN_MONTH_USD", "50")?,
            recheck: seconds("VIDEO_GEN_RECHECK_SECONDS", 1_800)?,
            retry_base: seconds("VIDEO_GEN_RETRY_SECONDS", 900)?,
            max_faults: bounded("VIDEO_GEN_MAX_FAULTS", 4, 1, 100)? as i32,
            shots: bounded("VIDEO_GEN_SHOTS", 6, 1, 12)? as usize,
            keyframe_tries: bounded("VIDEO_GEN_KEYFRAME_TRIES", 3, 1, 10)? as i32,
            shot_tries: bounded("VIDEO_GEN_SHOT_TRIES", 2, 1, 10)? as i32,
            side: bounded("VIDEO_GEN_SIDE", 720, 64, 1280)?,
            fps: bounded("VIDEO_GEN_FPS", 16, 1, 60)?,
            frames,
            stills: bounded("VIDEO_GEN_STILLS", 4, 1, 8)?,
            director,
        })
    }

    pub fn shot_ms(&self) -> i32 {
        (u64::from(self.frames) * 1000 / u64::from(self.fps)) as i32
    }
}

fn seconds(key: &str, default: u64) -> Result<Duration, ConfigError> {
    Ok(Duration::from_secs(env::positive_u64(key, default)?))
}

fn bounded(key: &str, default: u32, min: u32, max: u32) -> Result<u32, ConfigError> {
    let value: u32 = env::parse(key, &default.to_string())?;
    if !(min..=max).contains(&value) {
        return Err(ConfigError::Invalid {
            key: key.to_owned(),
            reason: format!("must be from {min} to {max}"),
        });
    }
    Ok(value)
}
