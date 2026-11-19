use std::path::Path;

use llm_ladder::{Kind, ProviderSettings, Rung, Settings};

use super::ai;
use super::env::{self, ConfigError};

#[derive(Clone, Debug)]
pub struct AnthropicLadder {
    pub steps: Vec<(String, ProviderSettings)>,
    pub reset_day: u32,
}

pub(super) fn load(
    key: &str,
    default: Option<&str>,
) -> Result<Option<AnthropicLadder>, ConfigError> {
    let Some(ladder) = env::optional(key).or_else(|| default.map(str::to_owned)) else {
        return Ok(None);
    };
    let path = env::required("LLM_CONFIG")?;
    let settings = ai::load(Path::new(&path))?;
    ladder_of(&settings, &ladder, key).map(Some)
}

pub(super) fn ladder_of(
    settings: &Settings,
    ladder: &str,
    key: &str,
) -> Result<AnthropicLadder, ConfigError> {
    let invalid = |reason: String| ConfigError::Invalid {
        key: key.to_owned(),
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
                    "provider {id} is not anthropic_compatible; this ladder has no other kind"
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
    Ok(AnthropicLadder {
        steps,
        reset_day: settings
            .budget
            .as_ref()
            .map_or(1, |budget| budget.reset_day),
    })
}

pub(super) fn month_usd(key: &str, default: &str) -> Result<f64, ConfigError> {
    let value: f64 = env::parse(key, default)?;
    if !value.is_finite() || value < 0.0 {
        return Err(ConfigError::Invalid {
            key: key.to_owned(),
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
    fn a_ladder_takes_its_providers_and_the_shared_reset_day() {
        let ladder = ladder_of(&settings(), "video", "KEY").unwrap();

        assert_eq!(ladder.steps.len(), 1);
        assert_eq!(ladder.steps[0].0, "opus");
        assert_eq!(ladder.reset_day, 17);
    }

    #[test]
    fn a_local_model_is_never_a_step_and_an_absent_ladder_is_refused() {
        assert!(ladder_of(&settings(), "mixed", "KEY").is_err());
        assert!(ladder_of(&settings(), "absent", "KEY").is_err());
    }
}
