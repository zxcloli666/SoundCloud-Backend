use std::time::Duration;

use llm_ladder::ProviderSettings;
use redact::Secret;
use serde::Deserialize;
use serde_json::{Map, Value, json};
use tracing::warn;

use crate::config::AnthropicLadder;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const MICRO: f64 = 1_000_000.0;

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
pub enum LlmError {
    #[error("the model API is unreachable: {0}")]
    Down(String),
    #[error("the model answered something unusable: {0}")]
    Invalid(String),
}

pub struct Answer<T> {
    pub result: Result<T, LlmError>,
    pub spend: Vec<Spend>,
}

pub struct Anthropic {
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
pub struct Message {
    pub content: Vec<Value>,
    pub stop_reason: Option<String>,
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

pub struct Call<'a> {
    pub name: &'static str,
    pub system: Option<&'a str>,
    pub messages: &'a [Value],
    pub tools: Option<Value>,
    pub format: Option<Value>,
    pub effort: Option<&'static str>,
}

impl Anthropic {
    pub fn new(ladder: &AnthropicLadder) -> Result<Self, wreq::Error> {
        let http = wreq::Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .build()?;
        let steps = ladder
            .steps
            .iter()
            .filter_map(|(id, settings)| step(id, settings))
            .collect();
        Ok(Self { http, steps })
    }

    pub fn is_empty(&self) -> bool {
        self.steps.is_empty()
    }

    pub async fn send(&self, call: &Call<'_>, spend: &mut Vec<Spend>) -> Result<Message, LlmError> {
        let mut last = LlmError::Down("no provider of the ladder has its key".to_owned());
        for step in &self.steps {
            match self.send_to(step, call).await {
                Ok(message) => {
                    spend.push(step.spend(call.name, &message.usage));
                    return Ok(message);
                }
                Err(LlmError::Down(reason)) => {
                    warn!(provider = %step.id, call = call.name, %reason, "the model API is unreachable");
                    last = LlmError::Down(reason);
                }
                Err(invalid) => return Err(invalid),
            }
        }
        Err(last)
    }

    async fn send_to(&self, step: &Step, call: &Call<'_>) -> Result<Message, LlmError> {
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
            .body(serde_json::to_vec(&body).map_err(|error| LlmError::Invalid(error.to_string()))?)
            .send()
            .await
            .map_err(|error| LlmError::Down(format!("transport: {error}")))?;
        let status = response.status().as_u16();
        let bytes = response
            .bytes()
            .await
            .map_err(|error| LlmError::Down(format!("body: {error}")))?;
        answer_of(status, &bytes)
    }
}

pub fn text_of(message: &Message) -> String {
    message
        .content
        .iter()
        .filter(|block| block["type"] == "text")
        .filter_map(|block| block["text"].as_str())
        .collect()
}

pub fn finished(message: &Message) -> Result<(), LlmError> {
    match message.stop_reason.as_deref() {
        Some("refusal") => Err(LlmError::Invalid("the model refused".to_owned())),
        Some("max_tokens") => Err(LlmError::Invalid("the answer was cut".to_owned())),
        _ => Ok(()),
    }
}

fn answer_of(status: u16, bytes: &[u8]) -> Result<Message, LlmError> {
    match status {
        200..=299 => serde_json::from_slice(bytes)
            .map_err(|error| LlmError::Invalid(format!("answer: {error}"))),
        401 | 403 | 429 | 500..=599 => Err(LlmError::Down(format!("status {status}"))),
        _ => Err(LlmError::Invalid(format!(
            "status {status}: {}",
            String::from_utf8_lossy(bytes)
                .chars()
                .take(300)
                .collect::<String>()
        ))),
    }
}

impl Step {
    fn body(&self, call: &Call<'_>) -> Value {
        let mut body = json!({
            "model": self.model,
            "max_tokens": self.max_tokens,
            "thinking": {"type": "adaptive"},
            "messages": call.messages,
        });
        if let Some(text) = call.system {
            let mut system = json!({"type": "text", "text": text});
            if let Some(mark) = &self.cache {
                system["cache_control"] = Value::Object(mark.clone());
            }
            body["system"] = json!([system]);
        }
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
        let mut output = Map::new();
        if let Some(schema) = &call.format {
            output.insert(
                "format".to_owned(),
                json!({"type": "json_schema", "schema": schema}),
            );
        }
        if let Some(effort) = call.effort {
            output.insert("effort".to_owned(), json!(effort));
        }
        if !output.is_empty() {
            body["output_config"] = Value::Object(output);
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
        warn!(provider = %id, "a model provider is off: its key is missing");
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
#[path = "anthropic_tests.rs"]
mod tests;
