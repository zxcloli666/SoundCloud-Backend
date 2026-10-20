use std::fs;
use std::path::Path;
use std::time::Duration;

use anyhow::{Context, ensure};
use redact::Secret;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::modules::chat::config::ModerationConfig;
use crate::modules::chat::moderation::CALL_TIMEOUT_SECS;
use crate::modules::chat::moderation::verdict::{self, Verdict};

pub const KEY_FILE: &str = "mod-ai-key";
pub const ATTEMPTS: u32 = 3;
pub const CALL_TIMEOUT: Duration = Duration::from_secs(CALL_TIMEOUT_SECS);
pub const BACKOFF: Duration = Duration::from_secs(1);
pub const ANSWER_TOKENS_MAX: u32 = 400;

pub struct AiClient {
    endpoint: Option<Endpoint>,
    http: wreq::Client,
    timeout: Duration,
    backoff: Duration,
}

struct Endpoint {
    url: String,
    model: String,
    key: Secret<String>,
}

pub struct Prompt {
    pub system: String,
    pub user: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Answered,
    Unparsable,
    Refused,
    Unavailable,
    Disabled,
}

impl Outcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Outcome::Answered => "answered",
            Outcome::Unparsable => "unparsable",
            Outcome::Refused => "refused",
            Outcome::Unavailable => "unavailable",
            Outcome::Disabled => "disabled",
        }
    }
}

pub struct AiAnswer {
    pub verdict: Option<Verdict>,
    pub answer_sha256: Option<String>,
    pub attempts: u32,
    pub outcome: Outcome,
}

enum Reply {
    Content(String),
    Final,
    Retry,
}

impl AiClient {
    pub fn load(config: &ModerationConfig, keys_dir: &Path) -> anyhow::Result<Self> {
        let http = wreq::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .build()
            .context("the moderation http client does not build")?;
        let mut client = Self {
            endpoint: None,
            http,
            timeout: CALL_TIMEOUT,
            backoff: BACKOFF,
        };
        if !config.enabled {
            return Ok(client);
        }
        let (Some(url), Some(model)) = (config.url.clone(), config.model.clone()) else {
            anyhow::bail!("CHAT_MOD_AI_ENABLED needs CHAT_MOD_AI_URL and CHAT_MOD_AI_MODEL");
        };
        let key = fs::read_to_string(keys_dir.join(KEY_FILE))
            .with_context(|| format!("{KEY_FILE} is not readable"))?;
        let key = key.trim().to_owned();
        ensure!(!key.is_empty(), "{KEY_FILE} is empty");
        client.endpoint = Some(Endpoint {
            url: completions_url(&url),
            model,
            key: Secret::new(key),
        });
        Ok(client)
    }

    pub fn enabled(&self) -> bool {
        self.endpoint.is_some()
    }

    pub fn model(&self) -> Option<&str> {
        self.endpoint
            .as_ref()
            .map(|endpoint| endpoint.model.as_str())
    }

    #[cfg(test)]
    pub fn with_timing(mut self, timeout: Duration, backoff: Duration) -> Self {
        self.timeout = timeout;
        self.backoff = backoff;
        self
    }

    pub async fn verdict(&self, prompt: &Prompt) -> AiAnswer {
        let Some(endpoint) = &self.endpoint else {
            return AiAnswer {
                verdict: None,
                answer_sha256: None,
                attempts: 0,
                outcome: Outcome::Disabled,
            };
        };
        let request = request_body(&endpoint.model, prompt);
        for attempt in 1..=ATTEMPTS {
            match self.call(endpoint, &request).await {
                Reply::Content(content) => {
                    let verdict = Verdict::parse(&content);
                    return AiAnswer {
                        outcome: if verdict.is_some() {
                            Outcome::Answered
                        } else {
                            Outcome::Unparsable
                        },
                        verdict,
                        answer_sha256: Some(sha256_hex(content.as_bytes())),
                        attempts: attempt,
                    };
                }
                Reply::Final => {
                    return AiAnswer {
                        verdict: None,
                        answer_sha256: None,
                        attempts: attempt,
                        outcome: Outcome::Refused,
                    };
                }
                Reply::Retry if attempt < ATTEMPTS => {
                    tokio::time::sleep(self.pause(attempt)).await;
                }
                Reply::Retry => {}
            }
        }
        AiAnswer {
            verdict: None,
            answer_sha256: None,
            attempts: ATTEMPTS,
            outcome: Outcome::Unavailable,
        }
    }

    fn pause(&self, attempt: u32) -> Duration {
        let jitter = Duration::from_millis(u64::from(rand::random::<u16>() % 250));
        self.backoff * 4u32.pow(attempt - 1) + jitter
    }

    async fn call(&self, endpoint: &Endpoint, request: &Value) -> Reply {
        let sent = self
            .http
            .post(&endpoint.url)
            .bearer_auth(endpoint.key.expose())
            .timeout(self.timeout)
            .json(request)
            .send()
            .await;
        let response = match sent {
            Ok(response) => response,
            Err(error) => {
                let kind = if error.is_timeout() {
                    "timeout"
                } else if error.is_connect() {
                    "connect"
                } else {
                    "transport"
                };
                tracing::warn!(kind, "moderation model call failed");
                return Reply::Retry;
            }
        };
        let status = response.status();
        if status.as_u16() == 429 || status.is_server_error() {
            tracing::warn!(status = status.as_u16(), "moderation model is busy");
            return Reply::Retry;
        }
        if !status.is_success() {
            tracing::warn!(
                status = status.as_u16(),
                "moderation model refused the call"
            );
            return Reply::Final;
        }
        match response.json::<Value>().await {
            Ok(answer) => answer["choices"][0]["message"]["content"]
                .as_str()
                .map_or(Reply::Final, |content| Reply::Content(content.to_owned())),
            Err(_) => Reply::Retry,
        }
    }
}

fn request_body(model: &str, prompt: &Prompt) -> Value {
    json!({
        "model": model,
        "temperature": 0,
        "max_tokens": ANSWER_TOKENS_MAX,
        "messages": [
            {"role": "system", "content": prompt.system},
            {"role": "user", "content": prompt.user}
        ],
        "response_format": {
            "type": "json_schema",
            "json_schema": {
                "name": "report_verdict",
                "strict": true,
                "schema": verdict::schema()
            }
        }
    })
}

pub fn completions_url(base: &str) -> String {
    let base = base.trim().trim_end_matches('/');
    if base.ends_with("/chat/completions") {
        base.to_owned()
    } else if base.ends_with("/v1") {
        format!("{base}/chat/completions")
    } else {
        format!("{base}/v1/chat/completions")
    }
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}
