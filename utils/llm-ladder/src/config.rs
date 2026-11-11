mod check;

use std::collections::BTreeMap;
use std::path::Path;

use redact::Secret;
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

use crate::error::ConfigError;
use crate::provider::worker::MAX_SYSTEM_BYTES as WORKER_MAX_SYSTEM_BYTES;

const OWN_SECTIONS: [&str; 3] = ["providers", "ladders", "budget"];
const SECRET_PREFIX: &str = "env:";
const MICRO: f64 = 1_000_000.0;

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    #[serde(default)]
    pub providers: BTreeMap<String, ProviderSettings>,
    #[serde(default)]
    pub ladders: BTreeMap<String, LadderSettings>,
    pub budget: Option<BudgetSettings>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderSettings {
    pub kind: Kind,
    pub endpoint: Option<String>,
    pub model: Option<String>,
    pub api_key: Option<SecretRef>,
    pub stream: Option<bool>,
    pub max_output_tokens: u32,
    pub total_timeout_ms: u64,
    pub hedge: Option<bool>,
    pub plain: Option<bool>,
    #[serde(default)]
    pub input_usd_per_mtok: f64,
    #[serde(default)]
    pub output_usd_per_mtok: f64,
    pub cache_read_usd_per_mtok: Option<f64>,
    pub cache_write_usd_per_mtok: Option<f64>,
    pub prefix_cache: Option<Map<String, Value>>,
    pub headers: Option<BTreeMap<String, String>>,
    pub body: Option<Map<String, Value>>,
    pub subject: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    AnthropicCompatible,
    OpenaiCompatible,
    Worker,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(try_from = "String")]
pub struct SecretRef {
    variable: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LadderSettings {
    pub steps: Vec<String>,
    pub hedge_after_ms: u64,
    pub deadline_ms: u64,
    pub breaker: BreakerSettings,
    #[serde(default)]
    pub budget: CapSettings,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BreakerSettings {
    pub failures: u32,
    pub window_s: u64,
    pub open_s: u64,
}

#[derive(Clone, Copy, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapSettings {
    pub month_usd: Option<f64>,
    pub day_usd: Option<f64>,
    pub user_month_usd: Option<f64>,
    pub user_day_usd: Option<f64>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BudgetSettings {
    pub bucket: String,
}

impl Settings {
    pub fn load<H: DeserializeOwned>(path: &Path) -> Result<(Self, H), ConfigError> {
        let text = std::fs::read_to_string(path).map_err(|error| ConfigError::Unreadable {
            path: path.display().to_string(),
            reason: error.to_string(),
        })?;
        Self::parse(&text)
    }

    pub fn parse<H: DeserializeOwned>(text: &str) -> Result<(Self, H), ConfigError> {
        let mut host: toml::Table = text.parse().map_err(syntax)?;
        let mut own = toml::Table::new();
        for section in OWN_SECTIONS {
            if let Some(value) = host.remove(section) {
                own.insert(section.to_owned(), value);
            }
        }
        let settings: Self = own.try_into().map_err(syntax)?;
        settings.validate()?;
        let host = host.try_into().map_err(syntax)?;
        Ok((settings, host))
    }

    pub fn max_system_bytes(&self, ladder: &str) -> Option<usize> {
        let steps = &self.ladders.get(ladder)?.steps;
        steps
            .iter()
            .filter_map(|step| self.providers.get(step))
            .any(|provider| provider.kind == Kind::Worker)
            .then_some(WORKER_MAX_SYSTEM_BYTES)
    }
}

impl SecretRef {
    pub(crate) fn resolve(&self) -> Option<Secret<String>> {
        std::env::var(&self.variable)
            .ok()
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty())
            .map(Secret::new)
    }
}

impl TryFrom<String> for SecretRef {
    type Error = String;

    fn try_from(raw: String) -> Result<Self, Self::Error> {
        match raw.strip_prefix(SECRET_PREFIX) {
            Some(variable) if !variable.is_empty() => Ok(Self {
                variable: variable.to_owned(),
            }),
            _ => Err("a secret is written as env:NAME, never as the value itself".to_owned()),
        }
    }
}

impl CapSettings {
    pub fn is_set(&self) -> bool {
        self.all().iter().any(Option::is_some)
    }

    fn all(&self) -> [Option<f64>; 4] {
        [
            self.month_usd,
            self.day_usd,
            self.user_month_usd,
            self.user_day_usd,
        ]
    }
}

pub(crate) fn micro_usd(usd: f64) -> u64 {
    (usd * MICRO).round() as u64
}

fn syntax(error: toml::de::Error) -> ConfigError {
    ConfigError::Syntax(error.message().to_owned())
}

#[cfg(test)]
#[path = "config_tests.rs"]
mod tests;
