use serde::Deserialize;
use serde_json::{Value, json};
use wreq::header::HeaderName;

use super::StepEvent;
use super::http::{Call, Wire};
use super::sse::{Read, Reader};
use crate::observer::Failure;
use crate::request::{Request, Usage};

const REFUSAL: &str = "refusal";

pub(crate) struct Messages;

impl Wire for Messages {
    type Reader = Events;

    const KEY_HEADER: HeaderName = HeaderName::from_static("x-api-key");
    const KEY_PREFIX: &'static str = "";
    const HEADERS: &'static [(&'static str, &'static str)] = &[("anthropic-version", "2026-10-21")];
    const SCHEMA_AT: &'static [&'static str] = &["output_config", "format", "schema"];

    fn body(call: &Call, request: &Request) -> Value {
        let mut body = json!({
            "model": call.model,
            "max_tokens": call.max_output_tokens,
            "system": request.system,
            "messages": request.messages,
            "output_config": {"format": {"type": "json_schema"}},
        });
        if call.stream {
            body["stream"] = Value::Bool(true);
        }
        body
    }

    fn whole(answer: &[u8]) -> Result<Vec<StepEvent>, Failure> {
        let message: Message = serde_json::from_slice(answer).map_err(|_| Failure::Invalid)?;
        if message.stop_reason.as_deref() == Some(REFUSAL) {
            return Err(Failure::Refused);
        }
        let mut usage = Usage::default();
        if let Some(tokens) = message.usage {
            tokens.add_to(&mut usage);
        }
        let text: String = message
            .content
            .into_iter()
            .filter(|block| block.kind == "text")
            .filter_map(|block| block.text)
            .collect();
        Ok(vec![StepEvent::Usage(usage), StepEvent::Text(text)])
    }
}

#[derive(Default)]
pub(crate) struct Events {
    usage: Usage,
}

impl Reader for Events {
    fn read(&mut self, data: &str) -> Read {
        let Ok(event) = serde_json::from_str::<Event>(data) else {
            return Read::Events(Vec::new());
        };
        match event.kind.as_str() {
            "message_start" => self.counted(event.message.and_then(|message| message.usage)),
            "content_block_delta" => Read::Events(
                event
                    .delta
                    .filter(|delta| delta.kind.as_deref() == Some("text_delta"))
                    .and_then(|delta| delta.text)
                    .filter(|text| !text.is_empty())
                    .map(StepEvent::Text)
                    .into_iter()
                    .collect(),
            ),
            "message_delta" if stopped_by_refusal(event.delta.as_ref()) => {
                Read::Failed(Failure::Refused)
            }
            "message_delta" => self.counted(event.usage),
            "message_stop" => Read::End,
            "error" => Read::Failed(Failure::Status),
            _ => Read::Events(Vec::new()),
        }
    }
}

impl Events {
    fn counted(&mut self, tokens: Option<Tokens>) -> Read {
        match tokens {
            Some(tokens) => {
                tokens.add_to(&mut self.usage);
                Read::Events(vec![StepEvent::Usage(self.usage)])
            }
            None => Read::Events(Vec::new()),
        }
    }
}

fn stopped_by_refusal(delta: Option<&Delta>) -> bool {
    delta.and_then(|delta| delta.stop_reason.as_deref()) == Some(REFUSAL)
}

#[derive(Deserialize)]
struct Event {
    #[serde(rename = "type")]
    kind: String,
    message: Option<Start>,
    delta: Option<Delta>,
    usage: Option<Tokens>,
}

#[derive(Deserialize)]
struct Start {
    usage: Option<Tokens>,
}

#[derive(Deserialize)]
struct Delta {
    #[serde(rename = "type")]
    kind: Option<String>,
    text: Option<String>,
    stop_reason: Option<String>,
}

#[derive(Deserialize)]
struct Tokens {
    input_tokens: Option<u64>,
    cache_creation_input_tokens: Option<u64>,
    cache_read_input_tokens: Option<u64>,
    output_tokens: Option<u64>,
}

#[derive(Deserialize)]
struct Message {
    #[serde(default)]
    content: Vec<Block>,
    usage: Option<Tokens>,
    stop_reason: Option<String>,
}

#[derive(Deserialize)]
struct Block {
    #[serde(rename = "type")]
    kind: String,
    text: Option<String>,
}

impl Tokens {
    fn add_to(&self, usage: &mut Usage) {
        if let Some(input) = self.input_tokens {
            usage.input_tokens = input
                + self.cache_creation_input_tokens.unwrap_or(0)
                + self.cache_read_input_tokens.unwrap_or(0);
        }
        if let Some(output) = self.output_tokens {
            usage.output_tokens = output;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read_all(lines: &[&str]) -> (Vec<StepEvent>, Option<Failure>, bool) {
        let mut reader = Events::default();
        let mut events = Vec::new();
        for line in lines {
            match reader.read(line) {
                Read::Events(more) => events.extend(more),
                Read::End => return (events, None, true),
                Read::Failed(failure) => return (events, Some(failure), false),
            }
        }
        (events, None, false)
    }

    #[test]
    fn text_deltas_and_cumulative_usage_are_read_and_the_rest_is_skipped() {
        let (events, failure, ended) = read_all(&[
            r#"{"type":"message_start","message":{"usage":{"input_tokens":1000,"cache_creation_input_tokens":10,"cache_read_input_tokens":5,"output_tokens":1}}}"#,
            r#"{"type":"content_block_start","index":0,"content_block":{"type":"thinking","thinking":""}}"#,
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"hidden"}}"#,
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"signature_delta","signature":"abc"}}"#,
            r#"{"type":"ping"}"#,
            r#"{"type":"content_block_delta","index":1,"delta":{"type":"text_delta","text":"{\"say\":"}}"#,
            r#"{"type":"content_block_delta","index":1,"delta":{"type":"text_delta","text":"\"hi\"}"}}"#,
            r#"{"type":"some_future_event","payload":1}"#,
            "not json at all",
            r#"{"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"input_tokens":1000,"output_tokens":100}}"#,
            r#"{"type":"message_stop"}"#,
        ]);
        assert_eq!(
            events,
            [
                StepEvent::Usage(Usage {
                    input_tokens: 1015,
                    output_tokens: 1
                }),
                StepEvent::Text("{\"say\":".to_owned()),
                StepEvent::Text("\"hi\"}".to_owned()),
                StepEvent::Usage(Usage {
                    input_tokens: 1000,
                    output_tokens: 100
                }),
            ]
        );
        assert!(failure.is_none() && ended);
    }

    #[test]
    fn a_whole_message_gives_its_text_blocks_and_usage() {
        let answer = br#"{"type":"message","content":[{"type":"thinking","thinking":"hidden"},{"type":"text","text":"{\"say\":"},{"type":"text","text":"\"hi\"}"}],"stop_reason":"end_turn","usage":{"input_tokens":1000,"output_tokens":100}}"#;
        assert_eq!(
            Messages::whole(answer),
            Ok(vec![
                StepEvent::Usage(Usage {
                    input_tokens: 1000,
                    output_tokens: 100
                }),
                StepEvent::Text("{\"say\":\"hi\"}".to_owned()),
            ])
        );
        let refused = br#"{"type":"message","content":[],"stop_reason":"refusal"}"#;
        assert_eq!(Messages::whole(refused), Err(Failure::Refused));
        assert_eq!(Messages::whole(b"<html>"), Err(Failure::Invalid));
    }

    #[test]
    fn the_request_names_the_model_the_limit_and_the_answer_format() {
        let call = Call {
            model: "example-model".to_owned(),
            max_output_tokens: 600,
            stream: true,
        };
        let request = Request {
            system: "rules".to_owned(),
            messages: vec![crate::request::Message::user("hello")],
            schema: crate::schema::Schema::object([("say", crate::schema::Schema::String)]),
            budget_user: None,
            deadline: None,
        };
        assert_eq!(
            Messages::body(&call, &request),
            json!({
                "model": "example-model",
                "max_tokens": 600,
                "system": "rules",
                "messages": [{"role": "user", "content": "hello"}],
                "output_config": {"format": {"type": "json_schema"}},
                "stream": true,
            })
        );
        let one_shot = Call {
            stream: false,
            ..call
        };
        assert!(Messages::body(&one_shot, &request).get("stream").is_none());
    }

    #[test]
    fn an_error_event_and_a_refusal_fail_the_step() {
        let (_, failure, _) = read_all(&[
            r#"{"type":"error","error":{"type":"overloaded_error","message":"Overloaded"}}"#,
        ]);
        assert_eq!(failure, Some(Failure::Status));
        let (_, failure, _) = read_all(&[
            r#"{"type":"message_delta","delta":{"stop_reason":"refusal"},"usage":{"output_tokens":3}}"#,
        ]);
        assert_eq!(failure, Some(Failure::Refused));
    }
}
