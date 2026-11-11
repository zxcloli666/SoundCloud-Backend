use std::iter::once;
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use futures::future::BoxFuture;
use futures::stream::{self, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{Backend, StepEvent, StepStream};
use crate::config::ProviderSettings;
use crate::observer::Failure;
use crate::request::{Message, Request, Usage};
use crate::schema::Schema;

pub(crate) const MAX_TOKENS: u32 = 1024;
pub(crate) const MAX_SYSTEM_BYTES: usize = 48_000;
const MAX_MESSAGES: usize = 64;
const MAX_MESSAGE_BYTES: usize = 12_000;
const MAX_SCHEMA_BYTES: usize = 64_000;

pub trait WorkerTransport: Send + Sync {
    fn call<'a>(
        &'a self,
        subject: &'a str,
        payload: Bytes,
        timeout: Duration,
    ) -> BoxFuture<'a, Result<Value, WorkerError>>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkerError {
    Timeout,
    Unreachable,
    Rejected,
    Invalid,
}

pub(crate) struct Worker {
    provider: String,
    transport: Arc<dyn WorkerTransport>,
    subject: Arc<str>,
    max_output_tokens: u32,
}

impl Worker {
    pub fn new(id: &str, settings: &ProviderSettings, transport: Arc<dyn WorkerTransport>) -> Self {
        Self {
            provider: id.to_owned(),
            transport,
            subject: settings.subject.clone().unwrap_or_default().into(),
            max_output_tokens: settings.max_output_tokens,
        }
    }
}

impl Backend for Worker {
    fn start(&self, request: Arc<Request>, time_left: Duration) -> StepStream {
        let provider = self.provider.clone();
        let transport = self.transport.clone();
        let subject = self.subject.clone();
        let max_tokens = self.max_output_tokens;
        stream::once(async move {
            let payload = Payload {
                system: request.system.joined(),
                messages: &request.messages,
                schema: &request.schema,
                max_tokens,
            };
            if let Some(part) = payload.over_limit() {
                tracing::warn!(provider = %provider, part, "llm worker takes no request this large");
                return vec![Err(Failure::Request)];
            }
            let Ok(payload) = serde_json::to_vec(&payload) else {
                return vec![Err(Failure::Request)];
            };
            match transport.call(&subject, payload.into(), time_left).await {
                Ok(data) => once(Ok(StepEvent::Accepted))
                    .chain(answered(&provider, data))
                    .collect(),
                Err(error) => {
                    tracing::warn!(provider = %provider, ?error, "llm worker did not answer");
                    vec![Err(Failure::from(error))]
                }
            }
        })
        .flat_map(stream::iter)
        .boxed()
    }
}

impl From<WorkerError> for Failure {
    fn from(error: WorkerError) -> Self {
        match error {
            WorkerError::Timeout => Self::Timeout,
            WorkerError::Unreachable => Self::Transport,
            WorkerError::Rejected => Self::Status,
            WorkerError::Invalid => Self::Request,
        }
    }
}

fn answered(provider: &str, data: Value) -> Vec<Result<StepEvent, Failure>> {
    let Ok(reply) = serde_json::from_value::<Reply>(data) else {
        tracing::warn!(provider, "llm worker answer has an unknown shape");
        return vec![Err(Failure::Transport)];
    };
    let usage = Usage {
        input_tokens: reply.input_tokens,
        output_tokens: reply.output_tokens,
        ..Usage::default()
    };
    vec![Ok(StepEvent::Usage(usage)), Ok(StepEvent::Text(reply.text))]
}

#[derive(Serialize)]
struct Payload<'a> {
    system: String,
    messages: &'a [Message],
    schema: &'a Schema,
    max_tokens: u32,
}

impl Payload<'_> {
    fn over_limit(&self) -> Option<&'static str> {
        let fits = |message: &Message| (1..=MAX_MESSAGE_BYTES).contains(&message.content.len());
        let schema = serde_json::to_vec(self.schema).map_or(usize::MAX, |schema| schema.len());
        if self.system.len() > MAX_SYSTEM_BYTES {
            Some("system")
        } else if !(1..=MAX_MESSAGES).contains(&self.messages.len()) {
            Some("messages")
        } else if !self.messages.iter().all(fits) {
            Some("message")
        } else if schema > MAX_SCHEMA_BYTES {
            Some("schema")
        } else {
            None
        }
    }
}

#[derive(Deserialize)]
struct Reply {
    text: String,
    #[serde(default)]
    input_tokens: u64,
    #[serde(default)]
    output_tokens: u64,
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::request::{Section, System};

    fn payload<'a>(system: &str, messages: &'a [Message], schema: &'a Schema) -> Payload<'a> {
        Payload {
            system: system.to_owned(),
            messages,
            schema,
            max_tokens: 512,
        }
    }

    #[test]
    fn the_request_is_the_contract_payload_with_the_schema_in_order() {
        let system = System {
            sections: vec![Section::shared("rules"), Section::own("state")],
        };
        let payload = Payload {
            system: system.joined(),
            messages: &[Message::user("hi"), Message::assistant("{\"say\":\"yo\"}")],
            schema: &Schema::object([("say", Schema::String), ("mood", Schema::Integer)]),
            max_tokens: 512,
        };
        let text = serde_json::to_string(&payload).unwrap();
        assert!(text.find("\"say\":{").unwrap() < text.find("\"mood\":{").unwrap());
        let value: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(value["system"], "rules\n\nstate");
        assert_eq!(value["max_tokens"], 512);
        assert_eq!(
            value["messages"],
            json!([
                {"role": "user", "content": "hi"},
                {"role": "assistant", "content": "{\"say\":\"yo\"}"},
            ])
        );
        assert_eq!(value["schema"]["type"], "object");
    }

    #[test]
    fn a_request_is_measured_against_the_limits_of_the_worker_method() {
        let schema = Schema::object([("say", Schema::String)]);
        let one = [Message::user("hi")];
        assert_eq!(payload("rules", &one, &schema).over_limit(), None);
        assert_eq!(
            payload(&"я".repeat(24_000), &one, &schema).over_limit(),
            None
        );
        assert_eq!(
            payload(&"я".repeat(24_001), &one, &schema).over_limit(),
            Some("system")
        );
        assert_eq!(
            payload("rules", &[], &schema).over_limit(),
            Some("messages")
        );
        let many = vec![Message::user("hi"); 65];
        assert_eq!(payload("rules", &many[..64], &schema).over_limit(), None);
        assert_eq!(
            payload("rules", &many, &schema).over_limit(),
            Some("messages")
        );
        let long = [Message::user("я".repeat(6_001))];
        assert_eq!(
            payload("rules", &long, &schema).over_limit(),
            Some("message")
        );
        let blank = [Message::user("")];
        assert_eq!(
            payload("rules", &blank, &schema).over_limit(),
            Some("message")
        );
        let names: Vec<String> = (0..5_000).map(|index| format!("choice-{index}")).collect();
        let wide = Schema::object([("say", Schema::one_of(names.iter().map(String::as_str)))]);
        assert_eq!(payload("rules", &one, &wide).over_limit(), Some("schema"));
    }

    #[test]
    fn the_reply_carries_text_and_token_counts() {
        let events = answered(
            "local",
            json!({"text": "{\"say\":\"hi\"}", "model": "example-model", "input_tokens": 40, "output_tokens": 9}),
        );
        assert_eq!(
            events,
            [
                Ok(StepEvent::Usage(Usage {
                    input_tokens: 40,
                    output_tokens: 9,
                    ..Usage::default()
                })),
                Ok(StepEvent::Text("{\"say\":\"hi\"}".to_owned())),
            ]
        );
        assert_eq!(
            answered("local", json!({"model": "x"})),
            [Err(Failure::Transport)]
        );
    }

    #[test]
    fn only_faults_of_the_worker_count_against_it() {
        let blamed = [
            WorkerError::Timeout,
            WorkerError::Unreachable,
            WorkerError::Rejected,
            WorkerError::Invalid,
        ]
        .map(|error| Failure::from(error).blames_provider());
        assert_eq!(blamed, [true, true, true, false]);
    }
}
