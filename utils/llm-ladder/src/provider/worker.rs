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
use crate::request::{Message, Request, Role, Usage};
use crate::schema::Schema;

pub(crate) const MAX_TOKENS: u32 = 1024;
pub(crate) const MAX_SYSTEM_BYTES: usize = 48_000;
const MAX_SYSTEM_MARKS: usize = 4;
const MAX_MESSAGES: usize = 64;
const MAX_MESSAGE_BYTES: usize = 12_000;
const MAX_SCHEMA_BYTES: usize = 64_000;

pub const WORKER_LIMITS: WorkerLimits = WorkerLimits {
    max_tokens: MAX_TOKENS,
    system_bytes: MAX_SYSTEM_BYTES,
    system_marks: MAX_SYSTEM_MARKS,
    messages: MAX_MESSAGES,
    message_bytes: MAX_MESSAGE_BYTES,
    schema_bytes: MAX_SCHEMA_BYTES,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WorkerLimits {
    pub max_tokens: u32,
    pub system_bytes: usize,
    pub system_marks: usize,
    pub messages: usize,
    pub message_bytes: usize,
    pub schema_bytes: usize,
}

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
            let payload = Payload::of(&request, max_tokens);
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

fn fitted(messages: &[Message]) -> Vec<Fitted<'_>> {
    let older = messages.len().saturating_sub(MAX_MESSAGES);
    let newest = &messages[older..];
    let replies = match older {
        0 => 0,
        _ => newest
            .iter()
            .take_while(|message| message.role == Role::Assistant)
            .count(),
    };
    newest[replies..]
        .iter()
        .map(|message| Fitted {
            role: message.role,
            content: newest_part(&message.content),
        })
        .collect()
}

fn newest_part(content: &str) -> &str {
    let start = content.len().saturating_sub(MAX_MESSAGE_BYTES);
    &content[content.ceil_char_boundary(start)..]
}

#[derive(Serialize)]
struct Payload<'a> {
    system: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    system_marks: Vec<usize>,
    messages: Vec<Fitted<'a>>,
    schema: &'a Schema,
    max_tokens: u32,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
struct Fitted<'a> {
    role: Role,
    content: &'a str,
}

impl<'a> Payload<'a> {
    fn of(request: &'a Request, max_tokens: u32) -> Self {
        let mut system_marks = request.system.shared_ends();
        system_marks.truncate(MAX_SYSTEM_MARKS);
        Self {
            system: request.system.joined(),
            system_marks,
            messages: fitted(&request.messages),
            schema: &request.schema,
            max_tokens,
        }
    }

    fn over_limit(&self) -> Option<&'static str> {
        let schema = serde_json::to_vec(self.schema).map_or(usize::MAX, |schema| schema.len());
        if self.system.len() > MAX_SYSTEM_BYTES {
            Some("system")
        } else if self.messages.is_empty() {
            Some("messages")
        } else if self
            .messages
            .iter()
            .any(|message| message.content.is_empty())
        {
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
            system_marks: Vec::new(),
            messages: fitted(messages),
            schema,
            max_tokens: 512,
        }
    }

    fn asked(sections: Vec<Section>) -> Request {
        Request {
            system: System { sections },
            messages: vec![Message::user("hi"), Message::assistant("{\"say\":\"yo\"}")],
            schema: Schema::object([("say", Schema::String), ("mood", Schema::Integer)]),
            budget_user: None,
            deadline: None,
        }
    }

    fn sent(sections: Vec<Section>) -> Value {
        serde_json::to_value(Payload::of(&asked(sections), 512)).unwrap()
    }

    fn talk(messages: usize) -> Vec<Message> {
        (0..messages)
            .map(|index| match index % 2 {
                0 => Message::user(format!("ask {index}")),
                _ => Message::assistant(format!("reply {index}")),
            })
            .collect()
    }

    #[test]
    fn the_request_is_the_contract_payload_with_the_schema_in_order() {
        let request = asked(vec![Section::shared("rules"), Section::own("state")]);
        let text = serde_json::to_string(&Payload::of(&request, 512)).unwrap();
        assert!(text.find("\"say\":{").unwrap() < text.find("\"mood\":{").unwrap());
        let value: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(value["system"], "rules\n\nstate");
        assert_eq!(value["system_marks"], json!([5]));
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
    fn the_end_of_every_shared_prefix_is_marked_in_bytes_of_the_system_text() {
        let chat = sent(vec![
            Section::shared("правила"),
            Section::shared("character"),
            Section::own("state"),
        ]);
        assert_eq!(chat["system"], "правила\n\ncharacter\n\nstate");
        assert_eq!(chat["system_marks"], json!([14, 25]));

        let whole = sent(vec![Section::shared("rules"), Section::shared("character")]);
        assert_eq!(whole["system_marks"], json!([5, 16]));
        assert_eq!(whole["system"].as_str().map(str::len), Some(16));

        let quiet = sent(vec![
            Section::shared("rules"),
            Section::shared("character"),
            Section::own(""),
        ]);
        assert_eq!(quiet["system_marks"], whole["system_marks"]);
    }

    #[test]
    fn a_system_text_with_no_shared_prefix_goes_out_without_marks() {
        let own = sent(vec![Section::own("rules"), Section::own("state")]);
        assert_eq!(own.get("system_marks"), None);
        assert_eq!(own["system"], "rules\n\nstate");
    }

    #[test]
    fn only_the_first_four_shared_prefixes_are_marked() {
        let many = sent(
            ["a", "b", "c", "d", "e", "f"]
                .into_iter()
                .map(Section::shared)
                .collect(),
        );
        assert_eq!(many["system_marks"], json!([1, 4, 7, 10]));
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
        let replies = vec![Message::assistant("hi"); 65];
        assert_eq!(
            payload("rules", &replies, &schema).over_limit(),
            Some("messages"),
            "nothing is left of 65 replies with no question"
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
    fn a_talk_within_the_limits_goes_out_as_it_is() {
        let whole = [
            Message::assistant("a reply first"),
            Message::user("я".repeat(6_000)),
        ];
        let sent: Vec<(Role, &str)> = fitted(&whole)
            .iter()
            .map(|message| (message.role, message.content))
            .collect();
        let given: Vec<(Role, &str)> = whole
            .iter()
            .map(|message| (message.role, message.content.as_str()))
            .collect();
        assert_eq!(sent, given);
        assert_eq!(fitted(&talk(64)).len(), 64);
    }

    #[test]
    fn a_longer_talk_keeps_its_newest_messages_and_still_begins_with_the_user() {
        let talk_of_66 = talk(66);
        let from_a_question = fitted(&talk_of_66);
        assert_eq!(from_a_question.len(), 64);
        assert_eq!(
            from_a_question[0],
            Fitted {
                role: Role::User,
                content: "ask 2"
            }
        );
        assert_eq!(from_a_question[63].content, "reply 65");

        let talk_of_65 = talk(65);
        let from_a_reply = fitted(&talk_of_65);
        assert_eq!(from_a_reply.len(), 63);
        assert_eq!(from_a_reply[0].content, "ask 2");
        assert_eq!(from_a_reply[62].content, "ask 64");
    }

    #[test]
    fn a_longer_message_keeps_its_newest_part_from_a_whole_character() {
        assert_eq!(newest_part("hi"), "hi");
        let at_the_limit = "я".repeat(6_000);
        assert_eq!(newest_part(&at_the_limit), at_the_limit);
        let over = format!("old {}", "я".repeat(6_000));
        assert_eq!(newest_part(&over), at_the_limit);
        let split = format!("{}!", "я".repeat(6_000));
        assert_eq!(newest_part(&split), format!("{}!", "я".repeat(5_999)));
        assert_eq!(newest_part(&split).len(), 11_999);
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
