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
}

pub(crate) struct Worker {
    provider: String,
    transport: Arc<dyn WorkerTransport>,
    subject: Arc<str>,
    max_output_tokens: u32,
    timeout: Duration,
}

impl Worker {
    pub fn new(id: &str, settings: &ProviderSettings, transport: Arc<dyn WorkerTransport>) -> Self {
        Self {
            provider: id.to_owned(),
            transport,
            subject: settings.subject.clone().unwrap_or_default().into(),
            max_output_tokens: settings.max_output_tokens,
            timeout: Duration::from_millis(settings.total_timeout_ms),
        }
    }
}

impl Backend for Worker {
    fn start(&self, request: Arc<Request>) -> StepStream {
        let provider = self.provider.clone();
        let transport = self.transport.clone();
        let subject = self.subject.clone();
        let timeout = self.timeout;
        let max_tokens = self.max_output_tokens;
        stream::once(async move {
            let payload = Payload {
                system: request.system.joined(),
                messages: &request.messages,
                schema: &request.schema,
                max_tokens,
            };
            let Ok(payload) = serde_json::to_vec(&payload) else {
                return vec![Err(Failure::Invalid)];
            };
            match transport.call(&subject, payload.into(), timeout).await {
                Ok(data) => answered(&provider, data),
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
        }
    }
}

fn answered(provider: &str, data: Value) -> Vec<Result<StepEvent, Failure>> {
    let Ok(reply) = serde_json::from_value::<Reply>(data) else {
        tracing::warn!(provider, "llm worker answer has an unknown shape");
        return vec![Err(Failure::Invalid)];
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
            [Err(Failure::Invalid)]
        );
    }
}
