use serde::Deserialize;
use serde_json::{Value, json};
use wreq::header::HeaderName;

use super::StepEvent;
use super::fault;
use super::http::{Call, Wire};
use super::sse::{Read, Reader};
use crate::observer::Failure;
use crate::request::{BETWEEN_SECTIONS, Request, System, Usage};

const REFUSAL: &str = "refusal";
const CUT: &str = "max_tokens";
const MAX_CACHE_MARKS: usize = 4;

pub(crate) struct Messages;

impl Wire for Messages {
    type Reader = Events;

    const KEY_HEADER: HeaderName = HeaderName::from_static("x-api-key");
    const KEY_PREFIX: &'static str = "";
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

    fn whole(answer: &[u8]) -> Vec<Result<StepEvent, Failure>> {
        let Ok(message) = serde_json::from_slice::<Message>(answer) else {
            return vec![Err(Failure::Transport)];
        };
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
        let stop = stopped(message.stop_reason.as_deref()).map(Err);
        [Ok(StepEvent::Usage(usage)), Ok(StepEvent::Text(text))]
            .into_iter()
            .chain(stop)
            .collect()
    }
}

#[derive(Default)]
pub(crate) struct Events {
    usage: Usage,
    ended: bool,
    stop: Option<Failure>,
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
            "message_delta" => {
                let reason = event
                    .delta
                    .as_ref()
                    .and_then(|delta| delta.stop_reason.as_deref());
                self.ended |= reason.is_some();
                self.stop = stopped(reason);
                self.counted(event.usage)
            }
            "message_stop" => Read::End,
            "error" => Read::Failed(
                event
                    .error
                    .as_ref()
                    .map_or(Failure::Status, fault::of_error),
            ),
            _ => Read::Events(Vec::new()),
        }
    }

    fn ended(&self) -> bool {
        self.ended
    }

    fn stop(&self) -> Option<Failure> {
        self.stop
    }
}

impl Events {
    fn counted(&mut self, tokens: Option<Tokens>) -> Read {
        match tokens {
            Some(tokens) => {
                tokens.add_to(&mut self.usage);
                Read::Events(vec![match self.ended {
                    true => StepEvent::Usage(self.usage),
                    false => StepEvent::UsageSoFar(self.usage),
                }])
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

fn stopped(reason: Option<&str>) -> Option<Failure> {
    match reason? {
        REFUSAL => Some(Failure::Refused),
        CUT => Some(Failure::Length),
        _ => None,
    }
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
    error: Option<Value>,
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
                Read::End => return (events, reader.stop(), true),
                Read::Failed(failure) => return (events, Some(failure), false),
            }
        }
        (events, reader.stop(), false)
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
                StepEvent::UsageSoFar(Usage {
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
    fn a_message_has_ended_once_its_stop_reason_came() {
        let mut reader = Events::default();
        reader.read(
            r#"{"type":"message_start","message":{"usage":{"input_tokens":40,"output_tokens":1}}}"#,
        );
        reader.read(
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"{}"}}"#,
        );
        reader.read(r#"{"type":"message_delta","delta":{},"usage":{"output_tokens":2}}"#);
        assert!(!reader.ended());
        reader.read(r#"{"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":3}}"#);
        assert!(reader.ended() && reader.stop().is_none());
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
            [
                Ok(StepEvent::Usage(Usage {
                    input_tokens: 40,
                    cache_read_tokens: 3000,
                    cache_write_tokens: 200,
                    output_tokens: 100
                })),
                Ok(StepEvent::Text("{\"say\":\"hi\"}".to_owned())),
            ]
        );
        let refused = br#"{"type":"message","content":[],"stop_reason":"refusal","usage":{"input_tokens":40,"output_tokens":3}}"#;
        let usage = Usage {
            input_tokens: 40,
            output_tokens: 3,
            ..Usage::default()
        };
        assert_eq!(
            Messages::whole(refused),
            [
                Ok(StepEvent::Usage(usage)),
                Ok(StepEvent::Text(String::new())),
                Err(Failure::Refused)
            ]
        );
        let cut = br#"{"type":"message","content":[{"type":"text","text":"{\"say\":\"ha"}],"stop_reason":"max_tokens"}"#;
        assert_eq!(Messages::whole(cut).last(), Some(&Err(Failure::Length)));
        assert_eq!(Messages::whole(b"<html>"), [Err(Failure::Transport)]);
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
    fn an_error_event_fails_the_step_and_names_whose_fault_it_is() {
        let (_, failure, _) = read_all(&[
            r#"{"type":"error","error":{"type":"overloaded_error","message":"Overloaded"}}"#,
        ]);
        assert_eq!(failure, Some(Failure::Status));
        let (_, failure, _) = read_all(&[
            r#"{"type":"error","error":{"type":"invalid_request_error","message":"prompt is too long"}}"#,
        ]);
        assert_eq!(failure, Some(Failure::Request));
    }

    #[test]
    fn a_refusal_or_a_cut_is_counted_and_then_fails_the_step_at_the_end() {
        for (reason, expected) in [
            ("refusal", Failure::Refused),
            ("max_tokens", Failure::Length),
        ] {
            let delta = format!(
                r#"{{"type":"message_delta","delta":{{"stop_reason":"{reason}"}},"usage":{{"output_tokens":3}}}}"#
            );
            let (events, failure, ended) = read_all(&[
                r#"{"type":"message_start","message":{"usage":{"input_tokens":40,"output_tokens":1}}}"#,
                &delta,
                r#"{"type":"message_stop"}"#,
            ]);
            assert_eq!(
                events.last(),
                Some(&StepEvent::Usage(Usage {
                    input_tokens: 40,
                    output_tokens: 3,
                    ..Usage::default()
                }))
            );
            assert_eq!((failure, ended), (Some(expected), true));
        }
        let (_, failure, ended) = read_all(&[
            r#"{"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":3}}"#,
            r#"{"type":"message_stop"}"#,
        ]);
        assert_eq!((failure, ended), (None, true));
    }
}
