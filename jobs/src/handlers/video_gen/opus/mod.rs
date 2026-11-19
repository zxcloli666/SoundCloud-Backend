mod review;

use std::time::Duration;

use llm_ladder::ProviderSettings;
use redact::Secret;
use serde::Deserialize;
use serde_json::{Map, Value, json};
use tracing::warn;

use super::director::{Answer, Brief, Director, DirectorError, Plan, Review, Spend, Verdict};
use super::prompts;
use crate::config::DirectorConfig;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const MICRO: f64 = 1_000_000.0;

pub struct Opus {
    http: wreq::Client,
    steps: Vec<Step>,
}

struct Step {
    id: String,
    endpoint: String,
    model: String,
    key: Secret<String>,
    headers: Vec<(String, String)>,
    extra: Map<String, Value>,
    cache: Option<Map<String, Value>>,
    max_tokens: u32,
    timeout: Duration,
    prices: Prices,
}

#[derive(Clone, Copy)]
struct Prices {
    input: f64,
    output: f64,
    cache_read: f64,
    cache_write: f64,
}

#[derive(Debug, Deserialize)]
pub(super) struct Message {
    pub(super) content: Vec<Value>,
    pub(super) stop_reason: Option<String>,
    #[serde(default)]
    usage: Usage,
}

#[derive(Debug, Default, Deserialize)]
struct Usage {
    #[serde(default)]
    input_tokens: i64,
    #[serde(default)]
    output_tokens: i64,
    #[serde(default)]
    cache_read_input_tokens: i64,
    #[serde(default)]
    cache_creation_input_tokens: i64,
}

pub(super) struct Call<'a> {
    pub(super) name: &'static str,
    pub(super) messages: &'a [Value],
    pub(super) tools: Option<Value>,
    pub(super) format: Option<Value>,
}

impl Opus {
    pub fn new(config: &DirectorConfig) -> Result<Self, wreq::Error> {
        let http = wreq::Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .build()?;
        let steps = config
            .steps
            .iter()
            .filter_map(|(id, settings)| step(id, settings))
            .collect();
        Ok(Self { http, steps })
    }

    pub(super) async fn send(
        &self,
        call: &Call<'_>,
        spend: &mut Vec<Spend>,
    ) -> Result<Message, DirectorError> {
        let mut last = DirectorError::Down("no director provider has its key".to_owned());
        for step in &self.steps {
            match self.send_to(step, call).await {
                Ok(message) => {
                    spend.push(step.spend(call.name, &message.usage));
                    return Ok(message);
                }
                Err(DirectorError::Down(reason)) => {
                    warn!(provider = %step.id, %reason, "video director is unreachable");
                    last = DirectorError::Down(reason);
                }
                Err(invalid) => return Err(invalid),
            }
        }
        Err(last)
    }

    async fn send_to(&self, step: &Step, call: &Call<'_>) -> Result<Message, DirectorError> {
        let body = step.body(call);
        let mut request = self
            .http
            .post(&step.endpoint)
            .timeout(step.timeout)
            .header("content-type", "application/json")
            .header("x-api-key", step.key.expose().as_str());
        for (name, value) in &step.headers {
            request = request.header(name.as_str(), value.as_str());
        }
        let response = request
            .body(
                serde_json::to_vec(&body)
                    .map_err(|error| DirectorError::Invalid(error.to_string()))?,
            )
            .send()
            .await
            .map_err(|error| DirectorError::Down(format!("transport: {error}")))?;
        let status = response.status().as_u16();
        let bytes = response
            .bytes()
            .await
            .map_err(|error| DirectorError::Down(format!("body: {error}")))?;
        match status {
            200..=299 => serde_json::from_slice(&bytes)
                .map_err(|error| DirectorError::Invalid(format!("answer: {error}"))),
            401 | 403 | 429 | 500..=599 => Err(DirectorError::Down(format!("status {status}"))),
            _ => Err(DirectorError::Invalid(format!(
                "status {status}: {}",
                String::from_utf8_lossy(&bytes)
                    .chars()
                    .take(300)
                    .collect::<String>()
            ))),
        }
    }
}

impl Director for Opus {
    async fn plan(&self, brief: &Brief) -> Answer<Plan> {
        let mut spend = Vec::new();
        let result = review::plan(self, brief, &mut spend).await;
        Answer { result, spend }
    }

    async fn review(&self, review: &Review<'_>) -> Answer<Verdict> {
        let mut spend = Vec::new();
        let result = review::verdict(self, review, &mut spend).await;
        Answer { result, spend }
    }
}

impl Step {
    fn body(&self, call: &Call<'_>) -> Value {
        let mut system = json!({"type": "text", "text": prompts::SYSTEM});
        if let Some(mark) = &self.cache {
            system["cache_control"] = Value::Object(mark.clone());
        }
        let mut body = json!({
            "model": self.model,
            "max_tokens": self.max_tokens,
            "system": [system],
            "thinking": {"type": "adaptive"},
            "messages": call.messages,
        });
        if let Some(tools) = &call.tools {
            let mut tools = tools.clone();
            if let (Some(mark), Some(last)) = (
                &self.cache,
                tools.as_array_mut().and_then(|all| all.last_mut()),
            ) {
                last["cache_control"] = Value::Object(mark.clone());
            }
            body["tools"] = tools;
            body["tool_choice"] = json!({"type": "auto"});
        }
        if let Some(schema) = &call.format {
            body["output_config"] = json!({"format": {"type": "json_schema", "schema": schema}});
        }
        merge(&mut body, &self.extra);
        body
    }

    fn spend(&self, call: &'static str, usage: &Usage) -> Spend {
        let usd = (usage.input_tokens as f64 * self.prices.input
            + usage.output_tokens as f64 * self.prices.output
            + usage.cache_read_input_tokens as f64 * self.prices.cache_read
            + usage.cache_creation_input_tokens as f64 * self.prices.cache_write)
            / MICRO;
        Spend {
            call,
            provider: self.id.clone(),
            model: self.model.clone(),
            input_tokens: usage.input_tokens,
            output_tokens: usage.output_tokens,
            cache_read_tokens: usage.cache_read_input_tokens,
            cache_write_tokens: usage.cache_creation_input_tokens,
            usd,
        }
    }
}

fn step(id: &str, settings: &ProviderSettings) -> Option<Step> {
    let key = settings.api_key.as_ref()?.resolve();
    let Some(key) = key else {
        warn!(provider = %id, "video director provider is off: its key is missing");
        return None;
    };
    Some(Step {
        id: id.to_owned(),
        endpoint: settings.endpoint.clone()?,
        model: settings.model.clone()?,
        key,
        headers: settings
            .headers
            .iter()
            .flatten()
            .map(|(name, value)| (name.clone(), value.clone()))
            .collect(),
        extra: settings.body.clone().unwrap_or_default(),
        cache: settings.prefix_cache.clone(),
        max_tokens: settings.max_output_tokens,
        timeout: Duration::from_millis(settings.total_timeout_ms),
        prices: Prices {
            input: settings.input_usd_per_mtok,
            output: settings.output_usd_per_mtok,
            cache_read: settings
                .cache_read_usd_per_mtok
                .unwrap_or(settings.input_usd_per_mtok),
            cache_write: settings
                .cache_write_usd_per_mtok
                .unwrap_or(settings.input_usd_per_mtok),
        },
    })
}

fn merge(base: &mut Value, extra: &Map<String, Value>) {
    let Value::Object(base) = base else {
        return;
    };
    for (key, value) in extra {
        match (base.get_mut(key), value) {
            (Some(existing @ Value::Object(_)), Value::Object(nested)) => merge(existing, nested),
            _ => {
                base.insert(key.clone(), value.clone());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opus() -> Step {
        Step {
            id: "opus".to_owned(),
            endpoint: "https://llm.example/v1/messages".to_owned(),
            model: "example-model".to_owned(),
            key: Secret::new("k".to_owned()),
            headers: Vec::new(),
            extra: serde_json::from_value(json!({"output_config": {"effort": "high"}})).unwrap(),
            cache: serde_json::from_value(json!({"type": "ephemeral", "ttl": "1h"})).unwrap(),
            max_tokens: 16_000,
            timeout: Duration::from_secs(300),
            prices: Prices {
                input: 5.0,
                output: 25.0,
                cache_read: 0.5,
                cache_write: 10.0,
            },
        }
    }

    #[test]
    fn the_system_prompt_and_the_last_tool_carry_the_cache_mark_and_config_extras_merge_in() {
        let tools = prompts::review_tools();
        let call = Call {
            name: "review",
            messages: &[],
            tools: Some(tools),
            format: Some(json!({"type": "object"})),
        };

        let body = opus().body(&call);

        assert_eq!(body["system"][0]["cache_control"]["ttl"], "1h");
        assert_eq!(body["tools"][2]["cache_control"]["type"], "ephemeral");
        assert!(body["tools"][0].get("cache_control").is_none());
        assert_eq!(body["tool_choice"]["type"], "auto");
        assert_eq!(body["thinking"]["type"], "adaptive");
        assert_eq!(body["output_config"]["effort"], "high");
        assert_eq!(body["output_config"]["format"]["type"], "json_schema");
    }

    #[test]
    fn spend_is_priced_from_real_usage_and_the_config_prices() {
        let usage = Usage {
            input_tokens: 2_000,
            output_tokens: 1_000,
            cache_read_input_tokens: 10_000,
            cache_creation_input_tokens: 1_000,
        };

        let spend = opus().spend("plan", &usage);

        assert!((spend.usd - (0.01 + 0.025 + 0.005 + 0.01)).abs() < 1e-9);
        assert_eq!(
            (spend.provider.as_str(), spend.cache_read_tokens),
            ("opus", 10_000)
        );
    }
}
