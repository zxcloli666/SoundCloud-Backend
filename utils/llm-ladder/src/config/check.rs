use std::collections::BTreeMap;

use serde_json::Map;
use wreq::header::{HeaderName, HeaderValue};

use super::{Kind, LadderSettings, ProviderSettings, Settings};
use crate::error::ConfigError;
use crate::provider::worker::MAX_TOKENS as WORKER_MAX_TOKENS;

const MAX_ID_LEN: usize = 32;
const MAX_WAIT_MS: u64 = 600_000;
const MAX_BREAKER_S: u64 = 86_400;
const RESERVED_HEADERS: [&str; 5] = [
    "authorization",
    "x-api-key",
    "content-type",
    "content-length",
    "host",
];

impl Settings {
    pub(super) fn validate(&self) -> Result<(), ConfigError> {
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
    fn validate(&self) -> Result<(), String> {
        if self.max_output_tokens == 0 || !(1..=MAX_WAIT_MS).contains(&self.total_timeout_ms) {
            return Err(format!(
                "needs max_output_tokens above zero and total_timeout_ms from 1 to {MAX_WAIT_MS}"
            ));
        }
        if !self.prices().into_iter().all(price) {
            return Err("has a price that is not a positive number".to_owned());
        }
        if let Some(headers) = &self.headers {
            headers_fit(headers)?;
        }
        let http = [
            self.endpoint.is_some(),
            self.model.is_some(),
            self.api_key.is_some(),
        ];
        let http_only = self.stream.is_some()
            || self.body.is_some()
            || self.prefix_cache.is_some()
            || self.headers.is_some();
        match self.kind {
            Kind::Worker if http.contains(&true) || http_only => Err(
                "is a worker and takes no endpoint, model, api_key, stream, headers, body or prefix_cache"
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

impl LadderSettings {
    fn validate(&self, providers: &BTreeMap<String, ProviderSettings>) -> Result<(), String> {
        if self.steps.is_empty() {
            return Err("has no steps".to_owned());
        }
        if !(1..=MAX_WAIT_MS).contains(&self.hedge_after_ms)
            || !(1..=MAX_WAIT_MS).contains(&self.deadline_ms)
        {
            return Err(format!(
                "needs hedge_after_ms and deadline_ms from 1 to {MAX_WAIT_MS}"
            ));
        }
        for (position, step) in self.steps.iter().enumerate() {
            let Some(provider) = providers.get(step) else {
                return Err(format!("names the unknown provider {step}"));
            };
            if self.steps[..position].contains(step) {
                return Err(format!("names the provider {step} twice"));
            }
            if provider.total_timeout_ms >= self.deadline_ms {
                return Err(format!(
                    "needs total_timeout_ms of {step} below its deadline_ms"
                ));
            }
        }
        let breaker = [self.breaker.window_s, self.breaker.open_s];
        if self.breaker.failures == 0
            || !breaker
                .iter()
                .all(|seconds| (1..=MAX_BREAKER_S).contains(seconds))
        {
            return Err(format!(
                "needs breaker failures above zero, window_s and open_s from 1 to {MAX_BREAKER_S}"
            ));
        }
        if !self.budget.all().into_iter().flatten().all(price) {
            return Err("has a spend cap that is not a positive number".to_owned());
        }
        Ok(())
    }
}

fn headers_fit(headers: &BTreeMap<String, String>) -> Result<(), String> {
    for (name, value) in headers {
        let parsed = HeaderName::from_bytes(name.as_bytes())
            .map_err(|_| format!("has a header name {name} that is not a header name"))?;
        if RESERVED_HEADERS.contains(&parsed.as_str()) {
            return Err(format!(
                "sets the header {name}, which the ladder sets itself"
            ));
        }
        if HeaderValue::from_str(value).is_err() {
            return Err(format!(
                "has a value of the header {name} that is not a header value"
            ));
        }
    }
    Ok(())
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

fn invalid(reason: String) -> ConfigError {
    ConfigError::Invalid(reason)
}
