use std::collections::BTreeMap;
use std::path::Path;

use redact::Secret;
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

use crate::error::ConfigError;
use crate::provider::worker::MAX_TOKENS as WORKER_MAX_TOKENS;

const OWN_SECTIONS: [&str; 3] = ["providers", "ladders", "budget"];
const SECRET_PREFIX: &str = "env:";
const MAX_ID_LEN: usize = 32;
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
    #[serde(default)]
    pub input_usd_per_mtok: f64,
    #[serde(default)]
    pub output_usd_per_mtok: f64,
    pub cache_read_usd_per_mtok: Option<f64>,
    pub cache_write_usd_per_mtok: Option<f64>,
    pub prefix_cache: Option<Map<String, Value>>,
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

    fn validate(&self) -> Result<(), ConfigError> {
        for (id, provider) in &self.providers {
            provider
                .validate()
                .and(identifier(id))
                .map_err(|reason| invalid(format!("provider {id} {reason}")))?;
        }
        for (id, ladder) in &self.ladders {
            ladder
                .validate(&self.providers)
                .and(identifier(id))
                .map_err(|reason| invalid(format!("ladder {id} {reason}")))?;
            if ladder.budget.is_set() && self.budget.is_none() {
                return Err(invalid(format!(
                    "ladder {id} has spend caps, so [budget] must name the bucket"
                )));
            }
        }
        match &self.budget {
            Some(budget) if identifier(&budget.bucket).is_err() => Err(invalid(
                "[budget] bucket is not a valid bucket name".to_owned(),
            )),
            _ => Ok(()),
        }
    }
}

impl ProviderSettings {
    pub fn priced(&self) -> bool {
        self.prices().into_iter().any(|usd| usd > 0.0)
    }

    fn validate(&self) -> Result<(), String> {
        if self.max_output_tokens == 0 || self.total_timeout_ms == 0 {
            return Err("needs max_output_tokens and total_timeout_ms above zero".to_owned());
        }
        if !self.prices().into_iter().all(price) {
            return Err("has a price that is not a positive number".to_owned());
        }
        let http = [
            self.endpoint.is_some(),
            self.model.is_some(),
            self.api_key.is_some(),
        ];
        let http_only = self.stream.is_some() || self.body.is_some() || self.prefix_cache.is_some();
        match self.kind {
            Kind::Worker if http.contains(&true) || http_only => Err(
                "is a worker and takes no endpoint, model, api_key, stream, body or prefix_cache"
                    .to_owned(),
            ),
            Kind::Worker if self.subject.as_deref().is_none_or(str::is_empty) => {
                Err("is a worker and needs a subject".to_owned())
            }
            Kind::Worker if self.max_output_tokens > WORKER_MAX_TOKENS => Err(format!(
                "is a worker and takes max_output_tokens up to {WORKER_MAX_TOKENS}"
            )),
            Kind::Worker => Ok(()),
            _ if http.contains(&false) => Err("needs endpoint, model and api_key".to_owned()),
            _ if self.subject.is_some() => Err("takes a subject only as a worker".to_owned()),
            _ if !self.endpoint.as_deref().is_some_and(is_http_url) => {
                Err("has an endpoint that is not an http(s) URL".to_owned())
            }
            Kind::OpenaiCompatible if self.prefix_cache.is_some() => {
                Err("takes prefix_cache only as anthropic_compatible".to_owned())
            }
            _ if self.prefix_cache.as_ref().is_some_and(Map::is_empty) => {
                Err("has an empty prefix_cache".to_owned())
            }
            _ => Ok(()),
        }
    }

    fn prices(&self) -> [f64; 4] {
        [
            self.input_usd_per_mtok,
            self.output_usd_per_mtok,
            self.cache_read_usd_per_mtok.unwrap_or(0.0),
            self.cache_write_usd_per_mtok.unwrap_or(0.0),
        ]
    }
}

impl SecretRef {
    pub fn variable(&self) -> &str {
        &self.variable
    }

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

impl LadderSettings {
    fn validate(&self, providers: &BTreeMap<String, ProviderSettings>) -> Result<(), String> {
        if self.steps.is_empty() {
            return Err("has no steps".to_owned());
        }
        for (position, step) in self.steps.iter().enumerate() {
            if !providers.contains_key(step) {
                return Err(format!("names the unknown provider {step}"));
            }
            if self.steps[..position].contains(step) {
                return Err(format!("names the provider {step} twice"));
            }
        }
        if self.hedge_after_ms == 0 || self.deadline_ms == 0 {
            return Err("needs hedge_after_ms and deadline_ms above zero".to_owned());
        }
        if self.breaker.failures == 0 || self.breaker.window_s == 0 || self.breaker.open_s == 0 {
            return Err("needs breaker failures, window_s and open_s above zero".to_owned());
        }
        if !self.budget.all().into_iter().flatten().all(price) {
            return Err("has a spend cap that is not a positive number".to_owned());
        }
        Ok(())
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

fn price(usd: f64) -> bool {
    usd.is_finite() && usd >= 0.0
}

fn is_http_url(endpoint: &str) -> bool {
    ["http://", "https://"].iter().any(|scheme| {
        endpoint
            .strip_prefix(scheme)
            .is_some_and(|rest| !rest.is_empty())
    })
}

fn identifier(id: &str) -> Result<(), String> {
    let plain = id.bytes().all(|byte| {
        byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_' || byte == b'-'
    });
    if id.is_empty() || id.len() > MAX_ID_LEN || !plain {
        return Err(format!(
            "has a name outside a-z, 0-9, _ and - or longer than {MAX_ID_LEN} characters"
        ));
    }
    Ok(())
}

fn syntax(error: toml::de::Error) -> ConfigError {
    ConfigError::Syntax(error.message().to_owned())
}

fn invalid(reason: String) -> ConfigError {
    ConfigError::Invalid(reason)
}

#[cfg(test)]
#[path = "config_tests.rs"]
mod tests;
