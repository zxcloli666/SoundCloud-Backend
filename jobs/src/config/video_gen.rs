use std::path::Path;
use std::time::Duration;

use llm_ladder::{Kind, ProviderSettings, Rung, Settings};

use super::ai;
use super::env::{self, ConfigError};

const DEFAULT_LADDER: &str = "video";
const LADDER_KEY: &str = "VIDEO_GEN_LADDER";

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
    pub director: Option<DirectorConfig>,
}

#[derive(Clone, Debug)]
pub struct DirectorConfig {
    pub steps: Vec<(String, ProviderSettings)>,
    pub reset_day: u32,
}

impl VideoGenConfig {
    pub(super) fn from_env() -> Result<Self, ConfigError> {
        let in_flight: i64 = env::parse("VIDEO_GEN_IN_FLIGHT", "0")?;
        let director = match in_flight > 0 {
            true => Some(director()?),
            false => None,
        };
        let frames = bounded("VIDEO_GEN_FRAMES", 81, 5, 161)?;
        Ok(Self {
            in_flight,
            tick: seconds("VIDEO_GEN_TICK_SECONDS", 30)?,
            timeout: seconds("VIDEO_GEN_TIMEOUT_SECONDS", 3_600)?,
            lease: seconds("VIDEO_GEN_LEASE_SECONDS", 900)?,
            month_usd: month_usd()?,
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

fn director() -> Result<DirectorConfig, ConfigError> {
    let path = env::required("LLM_CONFIG")?;
    let settings = ai::load(Path::new(&path))?;
    let ladder = env::optional("VIDEO_GEN_LADDER").unwrap_or_else(|| DEFAULT_LADDER.to_owned());
    director_of(&settings, &ladder)
}

pub(super) fn director_of(
    settings: &Settings,
    ladder: &str,
) -> Result<DirectorConfig, ConfigError> {
    let invalid = |reason: String| ConfigError::Invalid {
        key: LADDER_KEY.to_owned(),
        reason,
    };
    let Some(found) = settings.ladders.get(ladder) else {
        return Err(invalid(format!("LLM_CONFIG has no ladder {ladder}")));
    };
    let mut steps = Vec::new();
    for rung in &found.steps {
        let ids = match rung {
            Rung::One(id) => std::slice::from_ref(id),
            Rung::Together(ids) => ids.as_slice(),
        };
        for id in ids {
            let Some(provider) = settings.providers.get(id) else {
                return Err(invalid(format!(
                    "ladder {ladder} names unknown provider {id}"
                )));
            };
            if provider.kind != Kind::AnthropicCompatible {
                return Err(invalid(format!(
                    "provider {id} is not anthropic_compatible; the director has no other kind"
                )));
            }
            if provider.endpoint.is_none() || provider.model.is_none() || provider.api_key.is_none()
            {
                return Err(invalid(format!(
                    "provider {id} needs endpoint, model and api_key"
                )));
            }
            steps.push((id.clone(), provider.clone()));
        }
    }
    if steps.is_empty() {
        return Err(invalid(format!("ladder {ladder} has no providers")));
    }
    Ok(DirectorConfig {
        steps,
        reset_day: settings
            .budget
            .as_ref()
            .map_or(1, |budget| budget.reset_day),
    })
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

fn month_usd() -> Result<f64, ConfigError> {
    let value: f64 = env::parse("VIDEO_GEN_MONTH_USD", "50")?;
    if !value.is_finite() || value < 0.0 {
        return Err(ConfigError::Invalid {
            key: "VIDEO_GEN_MONTH_USD".to_owned(),
            reason: "must be a non-negative number".to_owned(),
        });
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    const LLM: &str = r#"
[providers.opus]
kind = "anthropic_compatible"
endpoint = "https://llm.example/v1/messages"
model = "example-model"
api_key = "env:EXAMPLE_KEY"
max_output_tokens = 16000
total_timeout_ms = 240000
input_usd_per_mtok = 5.0
output_usd_per_mtok = 25.0

[providers.local]
kind = "worker"
subject = "ai.rpc.complete"
max_output_tokens = 400
total_timeout_ms = 15000

[ladders.video]
steps = ["opus"]
hedge_after_ms = 300000
deadline_ms = 300000
breaker = { failures = 3, window_s = 60, open_s = 600 }

[ladders.mixed]
steps = ["opus", "local"]
hedge_after_ms = 300000
deadline_ms = 300000
breaker = { failures = 3, window_s = 60, open_s = 600 }

[budget]
bucket = "spend"
reset_day = 17
"#;

    fn settings() -> Settings {
        Settings::parse::<serde::de::IgnoredAny>(LLM).unwrap().0
    }

    #[test]
    fn the_director_takes_its_providers_and_the_shared_reset_day_from_the_ladder() {
        let director = director_of(&settings(), "video").unwrap();

        assert_eq!(director.steps.len(), 1);
        assert_eq!(director.steps[0].0, "opus");
        assert_eq!(director.reset_day, 17);
    }

    #[test]
    fn a_local_model_is_never_a_director() {
        assert!(director_of(&settings(), "mixed").is_err());
        assert!(director_of(&settings(), "absent").is_err());
    }
}
