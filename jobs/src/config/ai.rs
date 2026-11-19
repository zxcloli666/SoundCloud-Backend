use std::path::Path;

use llm_ladder::Settings;
use serde::Deserialize;

use super::env::{self, ConfigError};

const DEFAULT_LADDER: &str = "refine";

#[derive(Clone, Debug)]
pub struct AiConfig {
    pub refine: Option<RefineLadder>,
    pub timeout_ms: u64,
    pub daily_budget: u64,
}

#[derive(Clone, Debug)]
pub struct RefineLadder {
    pub settings: Settings,
    pub ladder: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NoHostSections {}

impl AiConfig {
    pub(super) fn from_env() -> Result<Self, ConfigError> {
        let enabled = env::boolean("ENRICH_AI_ENABLED", true)?;
        let ladder = env::optional("ENRICH_AI_LADDER").unwrap_or_else(|| DEFAULT_LADDER.to_owned());
        let settings = env::optional("LLM_CONFIG")
            .map(|path| load(Path::new(&path)))
            .transpose()?;
        Ok(Self {
            refine: refine(enabled, settings, ladder)?,
            timeout_ms: env::positive_u64("ENRICH_AI_TIMEOUT_MS", 20_000)?,
            daily_budget: env::positive_u64("ENRICH_AI_DAILY_BUDGET", 5_000)?,
        })
    }
}

pub(super) fn load(path: &Path) -> Result<Settings, ConfigError> {
    Settings::load::<NoHostSections>(path)
        .map(|(settings, _)| settings)
        .map_err(|error| ConfigError::Invalid {
            key: "LLM_CONFIG".to_owned(),
            reason: error.to_string(),
        })
}

fn refine(
    enabled: bool,
    settings: Option<Settings>,
    ladder: String,
) -> Result<Option<RefineLadder>, ConfigError> {
    let Some(settings) = settings.filter(|_| enabled) else {
        return Ok(None);
    };
    if !settings.ladders.contains_key(&ladder) {
        return Err(ConfigError::Invalid {
            key: "ENRICH_AI_LADDER".to_owned(),
            reason: format!("LLM_CONFIG has no ladder {ladder}"),
        });
    }
    Ok(Some(RefineLadder { settings, ladder }))
}

#[cfg(test)]
#[path = "ai_tests.rs"]
mod tests;
