use serde::Deserialize;
use serde_json::{Value, json};
use wreq::header::HeaderName;

use super::StepEvent;
use super::http::{Call, Wire};
use super::sse::{Read, Reader};
use crate::observer::Failure;
use crate::request::{BETWEEN_SECTIONS, Request, System, Usage};

const REFUSAL: &str = "refusal";
const MAX_CACHE_MARKS: usize = 4;

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
            "system": system(call, &request.system),
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

fn system(call: &Call, system: &System) -> Value {
    let blocks = blocks(system);
    match &call.prefix_cache {
        Some(mark) if blocks.iter().any(|block| block.marked) => blocks
            .into_iter()
            .map(|block| match block.marked {
                true => json!({"type": "text", "text": block.text, "cache_control": mark}),
                false => json!({"type": "text", "text": block.text}),
            })
            .collect(),
        _ => Value::String(system.joined()),
    }
}

fn blocks(system: &System) -> Vec<SystemBlock> {
    let mut blocks: Vec<SystemBlock> = Vec::new();
    let mut marks = 0;
    for section in &system.sections {
        if !section.text.is_empty() {
            let lead = match blocks.is_empty() {
                true => "",
                false => BETWEEN_SECTIONS,
            };
            blocks.push(SystemBlock {
                text: format!("{lead}{}", section.text),
                marked: false,
            });
        }
        if let Some(last) = blocks.last_mut()
            && section.ends_shared_prefix
            && !last.marked
            && marks < MAX_CACHE_MARKS
        {
            last.marked = true;
            marks += 1;
        }
    }
    blocks
}

fn stopped_by_refusal(delta: Option<&Delta>) -> bool {
    delta.and_then(|delta| delta.stop_reason.as_deref()) == Some(REFUSAL)
}

struct SystemBlock {
    text: String,
    marked: bool,
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
        let counts = [
            (self.input_tokens, &mut usage.input_tokens),
            (self.cache_read_input_tokens, &mut usage.cache_read_tokens),
            (
                self.cache_creation_input_tokens,
                &mut usage.cache_write_tokens,
            ),
            (self.output_tokens, &mut usage.output_tokens),
        ];
        for (reported, count) in counts {
            if let Some(reported) = reported {
                *count = reported;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::request::Section;

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

    fn call(prefix_cache: Option<Value>) -> Call {
        Call {
            model: "example-model".to_owned(),
            max_output_tokens: 600,
            stream: true,
            prefix_cache: prefix_cache.map(|mark| mark.as_object().cloned().unwrap_or_default()),
        }
    }

    fn request(system: System) -> Request {
        Request {
            system,
            messages: vec![crate::request::Message::user("hello")],
            schema: crate::schema::Schema::object([("say", crate::schema::Schema::String)]),
            budget_user: None,
            deadline: None,
        }
    }

    fn three_parts() -> System {
        System {
            sections: vec![
                Section::shared("rules"),
                Section::shared("character"),
                Section::own("state"),
            ],
        }
    }

    fn marked(text: &str) -> Value {
        json!({"type": "text", "text": text, "cache_control": {"type": "ephemeral"}})
    }

    fn plain(text: &str) -> Value {
        json!({"type": "text", "text": text})
    }

    fn sent(system: System) -> Value {
        let call = call(Some(json!({"type": "ephemeral"})));
        Messages::body(&call, &request(system))["system"].clone()
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
                    input_tokens: 1000,
                    cache_read_tokens: 5,
                    cache_write_tokens: 10,
                    output_tokens: 1
                }),
                StepEvent::Text("{\"say\":".to_owned()),
                StepEvent::Text("\"hi\"}".to_owned()),
                StepEvent::Usage(Usage {
                    input_tokens: 1000,
                    cache_read_tokens: 5,
                    cache_write_tokens: 10,
                    output_tokens: 100
                }),
            ]
        );
        assert!(failure.is_none() && ended);
    }

    #[test]
    fn the_last_reported_cache_counts_win() {
        let (events, _, _) = read_all(&[
            r#"{"type":"message_start","message":{"usage":{"input_tokens":40,"cache_creation_input_tokens":0,"cache_read_input_tokens":3000,"output_tokens":1}}}"#,
            r#"{"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"input_tokens":42,"cache_creation_input_tokens":7,"cache_read_input_tokens":3001,"output_tokens":70}}"#,
        ]);
        assert_eq!(
            events.last(),
            Some(&StepEvent::Usage(Usage {
                input_tokens: 42,
                cache_read_tokens: 3001,
                cache_write_tokens: 7,
                output_tokens: 70
            }))
        );
    }

    #[test]
    fn a_whole_message_gives_its_text_blocks_and_usage() {
        let answer = br#"{"type":"message","content":[{"type":"thinking","thinking":"hidden"},{"type":"text","text":"{\"say\":"},{"type":"text","text":"\"hi\"}"}],"stop_reason":"end_turn","usage":{"input_tokens":40,"cache_creation_input_tokens":200,"cache_read_input_tokens":3000,"output_tokens":100}}"#;
        assert_eq!(
            Messages::whole(answer),
            Ok(vec![
                StepEvent::Usage(Usage {
                    input_tokens: 40,
                    cache_read_tokens: 3000,
                    cache_write_tokens: 200,
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
        assert_eq!(
            Messages::body(&call(None), &request(three_parts())),
            json!({
                "model": "example-model",
                "max_tokens": 600,
                "system": "rules\n\ncharacter\n\nstate",
                "messages": [{"role": "user", "content": "hello"}],
                "output_config": {"format": {"type": "json_schema"}},
                "stream": true,
            })
        );
        let one_shot = Call {
            stream: false,
            ..call(None)
        };
        let body = Messages::body(&one_shot, &request(System::fixed("rules")));
        assert!(body.get("stream").is_none());
        assert_eq!(body["system"], "rules");
    }

    #[test]
    fn a_prefix_cache_marks_the_block_that_ends_each_shared_prefix() {
        let hour = call(Some(json!({"type": "ephemeral", "ttl": "1h"})));
        assert_eq!(
            Messages::body(&hour, &request(three_parts()))["system"],
            json!([
                {"type": "text", "text": "rules", "cache_control": {"type": "ephemeral", "ttl": "1h"}},
                {"type": "text", "text": "\n\ncharacter", "cache_control": {"type": "ephemeral", "ttl": "1h"}},
                {"type": "text", "text": "\n\nstate"},
            ])
        );
        assert_eq!(sent(System::fixed("rules")), json!([marked("rules")]));
    }

    #[test]
    fn a_shared_block_is_the_same_bytes_whatever_follows_it() {
        let with = |character: &str, state: &str| {
            sent(System {
                sections: vec![
                    Section::shared("rules"),
                    Section::shared(character),
                    Section::own(state),
                ],
            })
        };
        let first = with("loli", "volume 60");
        assert_eq!(first[0], with("loli", "volume 80")[0]);
        assert_eq!(first[1], with("loli", "volume 80")[1]);
        assert_eq!(first[1], with("loli", "")[1]);
        assert_eq!(with("loli", "").as_array().unwrap().len(), 2);
        assert_eq!(first[0], with("moriarty", "volume 60")[0]);
        assert_ne!(first[1], with("moriarty", "volume 60")[1]);
    }

    #[test]
    fn an_empty_section_sends_no_block_and_its_boundary_falls_on_the_block_before_it() {
        let empty_shared = System {
            sections: vec![
                Section::own("rules"),
                Section::shared(""),
                Section::own("state"),
            ],
        };
        assert_eq!(
            sent(empty_shared),
            json!([marked("rules"), plain("\n\nstate")])
        );
        let twice = System {
            sections: vec![
                Section::shared("rules"),
                Section::shared(""),
                Section::own("state"),
            ],
        };
        assert_eq!(sent(twice), json!([marked("rules"), plain("\n\nstate")]));
        let leading = System {
            sections: vec![Section::shared(""), Section::own("state")],
        };
        assert_eq!(sent(leading), "state");
    }

    #[test]
    fn a_text_with_no_shared_prefix_is_sent_as_one_string() {
        let own = System {
            sections: vec![Section::own("rules"), Section::own("state")],
        };
        assert_eq!(sent(own), "rules\n\nstate");
        assert_eq!(sent(System::default()), "");
    }

    #[test]
    fn only_the_first_four_shared_prefixes_are_marked() {
        let system = System {
            sections: ["a", "b", "c", "d", "e", "f"]
                .into_iter()
                .map(Section::shared)
                .collect(),
        };
        assert_eq!(
            sent(system),
            json!([
                marked("a"),
                marked("\n\nb"),
                marked("\n\nc"),
                marked("\n\nd"),
                plain("\n\ne"),
                plain("\n\nf"),
            ])
        );
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
