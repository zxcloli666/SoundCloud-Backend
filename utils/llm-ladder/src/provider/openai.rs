use serde::Deserialize;
use serde_json::{Value, json};
use wreq::header::{AUTHORIZATION, HeaderName};

use super::StepEvent;
use super::http::{Call, Wire};
use super::sse::{Read, Reader};
use crate::observer::Failure;
use crate::request::{Request, Usage};

const SCHEMA_NAME: &str = "answer";
const END: &str = "[DONE]";

pub(crate) struct ChatCompletions;

impl Wire for ChatCompletions {
    type Reader = Chunks;

    const KEY_HEADER: HeaderName = AUTHORIZATION;
    const KEY_PREFIX: &'static str = "Bearer ";
    const HEADERS: &'static [(&'static str, &'static str)] = &[];
    const SCHEMA_AT: &'static [&'static str] = &["response_format", "json_schema", "schema"];

    fn body(call: &Call, request: &Request) -> Value {
        let mut messages = vec![json!({"role": "system", "content": request.system.joined()})];
        messages.extend(request.messages.iter().map(|message| json!(message)));
        let mut body = json!({
            "model": call.model,
            "max_tokens": call.max_output_tokens,
            "messages": messages,
            "response_format": {
                "type": "json_schema",
                "json_schema": {"name": SCHEMA_NAME, "strict": true},
            },
        });
        if call.stream {
            body["stream"] = Value::Bool(true);
            body["stream_options"] = json!({"include_usage": true});
        }
        body
    }

    fn whole(answer: &[u8]) -> Result<Vec<StepEvent>, Failure> {
        match serde_json::from_slice::<Completion>(answer) {
            Ok(completion) if completion.error.is_some() => Err(Failure::Status),
            Ok(completion) => Ok(completion.events(|choice| choice.message)),
            Err(_) => Err(Failure::Invalid),
        }
    }
}

#[derive(Default)]
pub(crate) struct Chunks;

impl Reader for Chunks {
    fn read(&mut self, data: &str) -> Read {
        if data.trim() == END {
            return Read::End;
        }
        match serde_json::from_str::<Completion>(data) {
            Ok(completion) if completion.error.is_some() => Read::Failed(Failure::Status),
            Ok(completion) => Read::Events(completion.events(|choice| choice.delta)),
            Err(_) => Read::Events(Vec::new()),
        }
    }
}

#[derive(Deserialize)]
struct Completion {
    #[serde(default)]
    choices: Option<Vec<Choice>>,
    usage: Option<Tokens>,
    error: Option<Value>,
}

#[derive(Deserialize)]
struct Choice {
    delta: Option<Piece>,
    message: Option<Piece>,
}

#[derive(Deserialize)]
struct Piece {
    content: Option<Value>,
}

#[derive(Deserialize)]
struct Tokens {
    prompt_tokens: Option<u64>,
    completion_tokens: Option<u64>,
    prompt_tokens_details: Option<PromptDetails>,
}

#[derive(Deserialize)]
struct PromptDetails {
    cached_tokens: Option<u64>,
}

impl Completion {
    fn events(self, piece: impl Fn(Choice) -> Option<Piece>) -> Vec<StepEvent> {
        let usage = self.usage.map(|tokens| StepEvent::Usage(tokens.usage()));
        let text = self
            .choices
            .and_then(|choices| choices.into_iter().next())
            .and_then(piece)
            .and_then(|piece| piece.content)
            .and_then(|content| match content {
                Value::String(text) if !text.is_empty() => Some(StepEvent::Text(text)),
                _ => None,
            });
        usage.into_iter().chain(text).collect()
    }
}

impl Tokens {
    fn usage(&self) -> Usage {
        let prompt = self.prompt_tokens.unwrap_or(0);
        let cached = self
            .prompt_tokens_details
            .as_ref()
            .and_then(|details| details.cached_tokens)
            .unwrap_or(0)
            .min(prompt);
        Usage {
            input_tokens: prompt - cached,
            cache_read_tokens: cached,
            cache_write_tokens: 0,
            output_tokens: self.completion_tokens.unwrap_or(0),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read_all(lines: &[&str]) -> (Vec<StepEvent>, Option<Failure>, bool) {
        let mut reader = Chunks;
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
    fn content_is_read_and_reasoning_is_ignored() {
        let (events, failure, ended) = read_all(&[
            r#"{"choices":[{"index":0,"delta":{"role":"assistant","reasoning_content":"The user"},"finish_reason":null}],"usage":null}"#,
            r#"{"choices":[{"index":0,"delta":{"content":"{\"","role":"assistant","reasoning_content":"."},"finish_reason":null}],"usage":null}"#,
            r#"{"choices":[{"index":0,"delta":{"role":"assistant","content":""},"finish_reason":null}]}"#,
            r#"{"choices":[{"index":0,"delta":{"content":null,"reasoning":"more"},"finish_reason":null}]}"#,
            r#"{"choices":[{"index":0,"delta":{"content":"say\":\"hi\"}"},"finish_reason":"stop"}]}"#,
            r#"{"choices":[],"usage":{"prompt_tokens":90,"completion_tokens":120,"total_tokens":210}}"#,
            "[DONE]",
        ]);
        assert_eq!(
            events,
            [
                StepEvent::Text("{\"".to_owned()),
                StepEvent::Text("say\":\"hi\"}".to_owned()),
                StepEvent::Usage(Usage {
                    input_tokens: 90,
                    output_tokens: 120,
                    ..Usage::default()
                }),
            ]
        );
        assert!(failure.is_none() && ended);
    }

    #[test]
    fn cached_prompt_tokens_are_counted_apart_from_the_rest_of_the_prompt() {
        let (events, _, _) = read_all(&[
            r#"{"choices":[],"usage":{"prompt_tokens":3200,"completion_tokens":70,"prompt_tokens_details":{"cached_tokens":3000,"audio_tokens":0}}}"#,
        ]);
        assert_eq!(
            events,
            [StepEvent::Usage(Usage {
                input_tokens: 200,
                cache_read_tokens: 3000,
                cache_write_tokens: 0,
                output_tokens: 70
            })]
        );
        let (events, _, _) = read_all(&[
            r#"{"choices":[],"usage":{"prompt_tokens":90,"completion_tokens":7,"prompt_tokens_details":{"cached_tokens":500}}}"#,
            r#"{"choices":[],"usage":{"prompt_tokens":90,"completion_tokens":7,"prompt_tokens_details":null}}"#,
        ]);
        let prompts: Vec<u64> = events
            .iter()
            .map(|event| match event {
                StepEvent::Usage(usage) => usage.prompt_tokens(),
                StepEvent::Text(_) => 0,
            })
            .collect();
        assert_eq!(prompts, [90, 90]);
    }

    #[test]
    fn a_stream_without_usage_or_a_done_mark_still_reads() {
        let (events, failure, ended) = read_all(&[
            r#"{"choices":[{"delta":{"content":"{}"}}]}"#,
            ": comment that slipped through",
        ]);
        assert_eq!(events, [StepEvent::Text("{}".to_owned())]);
        assert!(failure.is_none() && !ended);
    }

    #[test]
    fn a_whole_completion_gives_its_message_and_usage() {
        let answer = br#"{"choices":[{"index":0,"message":{"role":"assistant","content":"```json\n{\"say\":\"hi\"}\n```","reasoning_content":"hidden"},"finish_reason":"stop"}],"usage":{"prompt_tokens":90,"completion_tokens":120}}"#;
        assert_eq!(
            ChatCompletions::whole(answer),
            Ok(vec![
                StepEvent::Usage(Usage {
                    input_tokens: 90,
                    output_tokens: 120,
                    ..Usage::default()
                }),
                StepEvent::Text("```json\n{\"say\":\"hi\"}\n```".to_owned()),
            ])
        );
        let failed = br#"{"error":{"message":"overloaded","code":429}}"#;
        assert_eq!(ChatCompletions::whole(failed), Err(Failure::Status));
        assert_eq!(ChatCompletions::whole(b"<html>"), Err(Failure::Invalid));
    }

    #[test]
    fn the_request_puts_the_system_text_first_and_asks_for_usage_when_streaming() {
        let call = Call {
            model: "example-model".to_owned(),
            max_output_tokens: 900,
            stream: true,
            prefix_cache: None,
        };
        let request = Request {
            system: crate::request::System {
                sections: vec![
                    crate::request::Section::shared("rules"),
                    crate::request::Section::own("state"),
                ],
            },
            messages: vec![
                crate::request::Message::user("hello"),
                crate::request::Message::assistant("hi"),
            ],
            schema: crate::schema::Schema::object([("say", crate::schema::Schema::String)]),
            budget_user: None,
            deadline: None,
        };
        assert_eq!(
            ChatCompletions::body(&call, &request),
            json!({
                "model": "example-model",
                "max_tokens": 900,
                "messages": [
                    {"role": "system", "content": "rules\n\nstate"},
                    {"role": "user", "content": "hello"},
                    {"role": "assistant", "content": "hi"},
                ],
                "response_format": {"type": "json_schema", "json_schema": {"name": "answer", "strict": true}},
                "stream": true,
                "stream_options": {"include_usage": true},
            })
        );
        let one_shot = ChatCompletions::body(
            &Call {
                stream: false,
                ..call
            },
            &request,
        );
        assert!(one_shot.get("stream").is_none() && one_shot.get("stream_options").is_none());
    }

    #[test]
    fn an_error_object_fails_the_step() {
        let (_, failure, _) = read_all(&[r#"{"error":{"message":"overloaded","code":429}}"#]);
        assert_eq!(failure, Some(Failure::Status));
    }
}
