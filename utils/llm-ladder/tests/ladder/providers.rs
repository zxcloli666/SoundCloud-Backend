use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::vendor::{self, COMPLETIONS, MESSAGES, Reply, Seen, Step, Vendor};
use crate::{ANSWER, NoHostSections};
use bytes::Bytes;
use futures::StreamExt;
use futures::future::BoxFuture;
use llm_ladder::{
    Completion, Event, Hosting, Ladder, LadderError, Ladders, Message, Outcome, Request, Schema,
    Section, Settings, System, Usage, WorkerError, WorkerTransport,
};
use serde::Deserialize;
use serde_json::{Value, json};

const KEY: &str = "llm-ladder";
const PIECES: [&str; 3] = ["{\"say\":", "\"hello ", "there\"}"];
const PIECE_GAP: Duration = Duration::from_millis(150);

#[derive(Debug, Deserialize, PartialEq)]
struct Said {
    say: String,
}

struct Bench {
    ladders: Ladders,
    seen: Arc<Seen>,
}

struct Answer {
    document: String,
    outcome: Result<Outcome, LadderError>,
    first_delta: Option<Duration>,
    finished: Duration,
    deltas: usize,
}

#[derive(Default)]
struct LocalModel {
    reply: Mutex<Option<Result<Value, WorkerError>>>,
    calls: Mutex<Vec<(String, Value, Duration)>>,
}

impl WorkerTransport for LocalModel {
    fn call<'a>(
        &'a self,
        subject: &'a str,
        payload: Bytes,
        timeout: Duration,
    ) -> BoxFuture<'a, Result<Value, WorkerError>> {
        Box::pin(async move {
            let payload = serde_json::from_slice(&payload).expect("the payload is JSON");
            self.calls
                .lock()
                .unwrap()
                .push((subject.to_owned(), payload, timeout));
            self.reply
                .lock()
                .unwrap()
                .clone()
                .expect("the test set a reply")
        })
    }
}

fn http_provider(id: &str, kind: &str, url: &str, model: &str, more: &str) -> String {
    format!(
        "[providers.{id}]\nkind = \"{kind}\"\nendpoint = \"{url}\"\nmodel = \"{model}\"\n\
         api_key = \"env:CARGO_PKG_NAME\"\nmax_output_tokens = 600\ntotal_timeout_ms = 4000\n{more}\n"
    )
}

fn ladder(id: &str, steps: &[&str], hedge_after_ms: u64) -> String {
    let steps: Vec<String> = steps.iter().map(|step| format!("\"{step}\"")).collect();
    format!(
        "[ladders.{id}]\nsteps = [{}]\nhedge_after_ms = {hedge_after_ms}\ndeadline_ms = 8000\n\
         breaker = {{ failures = 3, window_s = 30, open_s = 90 }}\n",
        steps.join(", ")
    )
}

fn bench(config: &str, worker: Option<Arc<dyn WorkerTransport>>) -> Bench {
    let (settings, _): (Settings, NoHostSections) =
        Settings::parse(config).expect("a valid config");
    let seen = Arc::new(Seen::default());
    let hosting = Hosting {
        jetstream: None,
        worker,
        observer: seen.clone(),
    };
    Bench {
        ladders: Ladders::build(&settings, hosting).expect("the ladders are built"),
        seen,
    }
}

fn request() -> Request {
    Request {
        system: System {
            sections: vec![
                Section::shared("Answer with one JSON object."),
                Section::shared("Speak as Loli."),
                Section::own("The volume is 60."),
            ],
        },
        messages: vec![
            Message::user("hi"),
            Message::assistant("hello"),
            Message::user("how are you"),
        ],
        schema: Schema::object([
            ("say", Schema::String),
            ("mood", Schema::one_of(["calm", "glad"])),
        ]),
        budget_user: None,
        deadline: None,
    }
}

async fn answer(ladder: &Arc<Ladder>) -> Answer {
    let started = Instant::now();
    let mut answer = Answer {
        document: String::new(),
        outcome: Err(LadderError::Unavailable),
        first_delta: None,
        finished: Duration::ZERO,
        deltas: 0,
    };
    match ladder.stream(request()).await {
        Ok(mut events) => {
            while let Some(event) = events.next().await {
                match event {
                    Ok(Event::Delta(delta)) => {
                        answer.first_delta.get_or_insert(started.elapsed());
                        answer.deltas += 1;
                        answer.document.push_str(&delta);
                    }
                    Ok(Event::Done(outcome)) => answer.outcome = Ok(outcome),
                    Err(error) => answer.outcome = Err(error),
                }
            }
        }
        Err(error) => answer.outcome = Err(error),
    }
    answer.finished = started.elapsed();
    answer
}

fn step(provider: &str, result: &'static str, usage: (u64, u64), cost_micro_usd: u64) -> Step {
    Step {
        provider: provider.to_owned(),
        result,
        usage: Usage {
            input_tokens: usage.0,
            output_tokens: usage.1,
            ..Usage::default()
        },
        cost_micro_usd,
    }
}

fn results(pairs: &[(&str, &'static str)]) -> Vec<(String, &'static str)> {
    pairs
        .iter()
        .map(|(provider, result)| ((*provider).to_owned(), *result))
        .collect()
}

#[tokio::test]
async fn a_messages_stream_is_asked_in_its_wire_shape_and_read_piece_by_piece() {
    let vendor = Vendor::open().await;
    vendor.model(
        "stream-model",
        Reply::Events(vendor::message_events(&PIECES, 1_000, 100)),
    );
    let config = http_provider(
        "paid",
        "anthropic_compatible",
        &vendor.url(MESSAGES),
        "stream-model",
        "input_usd_per_mtok = 2.0\noutput_usd_per_mtok = 8.0\nbody = { max_tokens = 300, output_config = { effort = \"low\" } }",
    ) + &ladder("chat", &["paid"], 500);
    let bench = bench(&config, None);

    let answer = answer(&bench.ladders.get("chat").unwrap()).await;

    let outcome = answer.outcome.unwrap();
    assert_eq!(answer.document, ANSWER);
    assert_eq!(answer.deltas, 3);
    assert!(
        answer.finished - answer.first_delta.unwrap() >= PIECE_GAP,
        "pieces arrive as they are sent, not all at the end: {answer_first:?} then {answer_end:?}",
        answer_first = answer.first_delta,
        answer_end = answer.finished
    );
    assert_eq!(outcome.provider, "paid");
    assert_eq!(
        outcome.usage,
        Usage {
            input_tokens: 1_000,
            output_tokens: 100,
            ..Usage::default()
        }
    );
    assert_eq!(outcome.cost_micro_usd, 2_800);
    assert_eq!(
        bench.seen.steps(),
        [step("paid", "won", (1_000, 100), 2_800)]
    );

    let calls = vendor.calls();
    assert_eq!(calls.len(), 1);
    let call = &calls[0];
    assert_eq!(call.path, MESSAGES);
    assert_eq!(call.header("x-api-key"), Some(KEY));
    assert_eq!(call.header("anthropic-version"), Some("2026-10-21"));
    assert_eq!(call.header("content-type"), Some("application/json"));
    assert_eq!(call.header("authorization"), None);
    assert_eq!(
        call.json(),
        json!({
            "model": "stream-model",
            "max_tokens": 300,
            "stream": true,
            "system": "Answer with one JSON object.\n\nSpeak as Loli.\n\nThe volume is 60.",
            "messages": [
                {"role": "user", "content": "hi"},
                {"role": "assistant", "content": "hello"},
                {"role": "user", "content": "how are you"},
            ],
            "output_config": {
                "effort": "low",
                "format": {
                    "type": "json_schema",
                    "schema": {
                        "type": "object",
                        "properties": {
                            "say": {"type": "string"},
                            "mood": {"type": "string", "enum": ["calm", "glad"]},
                        },
                        "required": ["say", "mood"],
                        "additionalProperties": false,
                    },
                },
            },
        })
    );
    assert!(
        call.body.find("\"say\"").unwrap() < call.body.find("\"mood\"").unwrap(),
        "the schema keeps the order the caller gave: {}",
        call.body
    );
}

#[tokio::test]
async fn a_prefix_cache_marks_the_shared_blocks_and_cached_input_has_its_own_prices() {
    let vendor = Vendor::open().await;
    let start = json!({
        "input_tokens": 40,
        "cache_creation_input_tokens": 200,
        "cache_read_input_tokens": 3_000,
        "output_tokens": 1,
    });
    vendor.model(
        "stream-model",
        Reply::Events(vendor::message_events_counting(&PIECES, start, 70)),
    );
    let config = http_provider(
        "paid",
        "anthropic_compatible",
        &vendor.url(MESSAGES),
        "stream-model",
        "input_usd_per_mtok = 2.0\noutput_usd_per_mtok = 8.0\n\
         cache_read_usd_per_mtok = 0.2\ncache_write_usd_per_mtok = 2.5\n\
         prefix_cache = { type = \"ephemeral\", ttl = \"1h\" }",
    ) + &ladder("chat", &["paid"], 500);
    let bench = bench(&config, None);

    let answer = answer(&bench.ladders.get("chat").unwrap()).await;

    let outcome = answer.outcome.unwrap();
    assert_eq!(answer.document, ANSWER);
    let usage = Usage {
        input_tokens: 40,
        cache_read_tokens: 3_000,
        cache_write_tokens: 200,
        output_tokens: 70,
    };
    assert_eq!(outcome.usage, usage);
    assert_eq!(outcome.cost_micro_usd, 80 + 600 + 500 + 560);
    assert_eq!(bench.seen.steps()[0].usage, usage);
    assert_eq!(bench.seen.steps()[0].cost_micro_usd, 1_740);
    let body = vendor.calls()[0].json();
    assert_eq!(
        body["system"],
        json!([
            {
                "type": "text",
                "text": "Answer with one JSON object.",
                "cache_control": {"type": "ephemeral", "ttl": "1h"},
            },
            {
                "type": "text",
                "text": "\n\nSpeak as Loli.",
                "cache_control": {"type": "ephemeral", "ttl": "1h"},
            },
            {"type": "text", "text": "\n\nThe volume is 60."},
        ])
    );
    assert_eq!(
        body["messages"][0],
        json!({"role": "user", "content": "hi"})
    );
}

#[tokio::test]
async fn cached_prompt_tokens_of_a_chat_completion_are_charged_as_cache_reads() {
    let vendor = Vendor::open().await;
    let completion = json!({
        "choices": [{"index": 0, "message": {"role": "assistant", "content": ANSWER}}],
        "usage": {
            "prompt_tokens": 3_200,
            "completion_tokens": 70,
            "prompt_tokens_details": {"cached_tokens": 3_000},
        },
    });
    vendor.model("whole-model", Reply::Whole(0, completion));
    let config = http_provider(
        "proxy",
        "openai_compatible",
        &vendor.url(COMPLETIONS),
        "whole-model",
        "stream = false\ninput_usd_per_mtok = 2.0\noutput_usd_per_mtok = 8.0\n\
         cache_read_usd_per_mtok = 0.2",
    ) + &ladder("chat", &["proxy"], 500);
    let bench = bench(&config, None);

    let outcome = answer(&bench.ladders.get("chat").unwrap())
        .await
        .outcome
        .unwrap();

    assert_eq!(
        outcome.usage,
        Usage {
            input_tokens: 200,
            cache_read_tokens: 3_000,
            cache_write_tokens: 0,
            output_tokens: 70,
        }
    );
    assert_eq!(outcome.cost_micro_usd, 400 + 600 + 560);
}

#[tokio::test]
async fn a_messages_one_shot_answer_comes_as_a_single_delta() {
    let vendor = Vendor::open().await;
    vendor.model(
        "whole-model",
        Reply::Whole(50, vendor::whole_message(ANSWER, 1_000, 100)),
    );
    let config = http_provider(
        "paid",
        "anthropic_compatible",
        &vendor.url(MESSAGES),
        "whole-model",
        "stream = false\ninput_usd_per_mtok = 2.0\noutput_usd_per_mtok = 8.0",
    ) + &ladder("chat", &["paid"], 500);
    let bench = bench(&config, None);

    let answer = answer(&bench.ladders.get("chat").unwrap()).await;

    assert_eq!((answer.document.as_str(), answer.deltas), (ANSWER, 1));
    assert_eq!(answer.outcome.unwrap().cost_micro_usd, 2_800);
    assert!(vendor.calls()[0].json().get("stream").is_none());
}

#[tokio::test]
async fn a_chat_completions_stream_is_asked_in_its_wire_shape_and_reasoning_is_dropped() {
    let vendor = Vendor::open().await;
    let fenced = ["```json\n{\"say\":", "\"hello ", "there\"}\n```"];
    vendor.model(
        "stream-model",
        Reply::Events(vendor::completion_chunks(&fenced, Some((90, 120)))),
    );
    let config = http_provider(
        "proxy",
        "openai_compatible",
        &vendor.url(COMPLETIONS),
        "stream-model",
        "body = { temperature = 0.2 }",
    ) + &ladder("chat", &["proxy"], 500);
    let bench = bench(&config, None);

    let answer = answer(&bench.ladders.get("chat").unwrap()).await;

    let outcome = answer.outcome.unwrap();
    assert_eq!(answer.document, ANSWER);
    assert_eq!(answer.deltas, 3);
    assert!(answer.finished - answer.first_delta.unwrap() >= PIECE_GAP);
    assert_eq!(
        outcome.usage,
        Usage {
            input_tokens: 90,
            output_tokens: 120,
            ..Usage::default()
        }
    );
    assert_eq!(
        outcome.cost_micro_usd, 0,
        "a provider without a price costs nothing"
    );

    let call = &vendor.calls()[0];
    assert_eq!(call.path, COMPLETIONS);
    assert_eq!(call.header("authorization"), Some("Bearer llm-ladder"));
    assert_eq!(call.header("x-api-key"), None);
    let body = call.json();
    assert_eq!(body["model"], "stream-model");
    assert_eq!(body["max_tokens"], 600);
    assert_eq!(body["temperature"], 0.2);
    assert_eq!(body["stream"], true);
    assert_eq!(body["stream_options"], json!({"include_usage": true}));
    assert_eq!(
        body["messages"],
        json!([
            {"role": "system", "content": "Answer with one JSON object.\n\nSpeak as Loli.\n\nThe volume is 60."},
            {"role": "user", "content": "hi"},
            {"role": "assistant", "content": "hello"},
            {"role": "user", "content": "how are you"},
        ])
    );
    assert_eq!(body["response_format"]["type"], "json_schema");
    assert_eq!(body["response_format"]["json_schema"]["strict"], true);
    assert_eq!(
        body["response_format"]["json_schema"]["schema"]["required"],
        json!(["say", "mood"])
    );
}

#[tokio::test]
async fn a_chat_completions_one_shot_answer_without_usage_still_reads() {
    let vendor = Vendor::open().await;
    let fenced = format!("Sure!\n```json\n{ANSWER}\n```\nAnything else?");
    vendor.model(
        "whole-model",
        Reply::Whole(50, vendor::whole_completion(&fenced, None)),
    );
    let config = http_provider(
        "proxy",
        "openai_compatible",
        &vendor.url(COMPLETIONS),
        "whole-model",
        "stream = false",
    ) + &ladder("chat", &["proxy"], 500);
    let bench = bench(&config, None);

    let completion: Completion<Said> = bench
        .ladders
        .get("chat")
        .unwrap()
        .complete(request())
        .await
        .unwrap();

    assert_eq!(completion.value.say, "hello there");
    assert_eq!(completion.outcome.usage, Usage::default());
    let body = vendor.calls()[0].json();
    assert!(body.get("stream").is_none() && body.get("stream_options").is_none());
}

#[tokio::test]
async fn a_refused_call_and_a_bad_answer_fall_through_to_the_next_step() {
    let vendor = Vendor::open().await;
    vendor.model(
        "down-model",
        Reply::Status(
            529,
            json!({"type": "error", "error": {"type": "overloaded_error", "message": "busy"}}),
        ),
    );
    vendor.model(
        "prose-model",
        Reply::Whole(
            0,
            vendor::whole_completion("I would rather chat in prose.", Some((5, 7))),
        ),
    );
    vendor.model(
        "good-model",
        Reply::Events(vendor::completion_chunks(&PIECES, None)),
    );
    let config = http_provider(
        "down",
        "anthropic_compatible",
        &vendor.url(MESSAGES),
        "down-model",
        "",
    ) + &http_provider(
        "prose",
        "openai_compatible",
        &vendor.url(COMPLETIONS),
        "prose-model",
        "stream = false",
    ) + &http_provider(
        "good",
        "openai_compatible",
        &vendor.url(COMPLETIONS),
        "good-model",
        "",
    ) + &ladder("chat", &["down", "prose", "good"], 3_000);
    let bench = bench(&config, None);

    let answer = answer(&bench.ladders.get("chat").unwrap()).await;

    assert_eq!(answer.document, ANSWER);
    assert_eq!(answer.outcome.unwrap().provider, "good");
    assert!(
        answer.finished < Duration::from_millis(2_500),
        "a failure does not wait for the hedge: {:?}",
        answer.finished
    );
    assert_eq!(
        bench.seen.results(),
        results(&[
            ("down", "failed_status"),
            ("prose", "failed_invalid"),
            ("good", "won")
        ])
    );
}

#[tokio::test]
async fn a_line_cut_in_the_middle_of_an_answer_fails_the_request() {
    let vendor = Vendor::open().await;
    let mut cut = vendor::message_events(&PIECES, 1_000, 100);
    cut.truncate(5);
    vendor.model("cut-model", Reply::Breaks(cut));
    vendor.model(
        "spare-model",
        Reply::Events(vendor::completion_chunks(&PIECES, None)),
    );
    let config = http_provider(
        "cut",
        "anthropic_compatible",
        &vendor.url(MESSAGES),
        "cut-model",
        "",
    ) + &http_provider(
        "spare",
        "openai_compatible",
        &vendor.url(COMPLETIONS),
        "spare-model",
        "",
    ) + &ladder("chat", &["cut", "spare"], 3_000);
    let bench = bench(&config, None);

    let answer = answer(&bench.ladders.get("chat").unwrap()).await;

    assert_eq!(answer.document, "{\"say\":\"hello ");
    assert_eq!(answer.outcome, Err(LadderError::ProviderFailed));
    assert_eq!(
        bench.seen.results(),
        results(&[("cut", "failed_transport")])
    );
    assert!(vendor.calls_of("spare-model").is_empty());
}

#[tokio::test]
async fn a_silent_provider_is_hedged_and_hung_up_on_once_the_other_speaks() {
    let vendor = Vendor::open().await;
    vendor.model(
        "silent-model",
        Reply::Whole(30_000, vendor::whole_message(ANSWER, 1, 1)),
    );
    vendor.model(
        "quick-model",
        Reply::Events(vendor::completion_chunks(&PIECES, None)),
    );
    let config = http_provider(
        "silent",
        "anthropic_compatible",
        &vendor.url(MESSAGES),
        "silent-model",
        "stream = false",
    ) + &http_provider(
        "quick",
        "openai_compatible",
        &vendor.url(COMPLETIONS),
        "quick-model",
        "",
    ) + &ladder("chat", &["silent", "quick"], 200);
    let bench = bench(&config, None);

    let answer = answer(&bench.ladders.get("chat").unwrap()).await;

    assert_eq!(answer.document, ANSWER);
    assert_eq!(answer.outcome.unwrap().provider, "quick");
    assert!(answer.first_delta.unwrap() >= Duration::from_millis(200));
    assert!(
        answer.finished < Duration::from_millis(3_500),
        "{:?}",
        answer.finished
    );
    assert_eq!(
        bench.seen.results(),
        results(&[("silent", "lost"), ("quick", "won")])
    );
    assert!(
        vendor.hung_up(1).await,
        "the losing call is closed on the wire"
    );
}

#[tokio::test]
async fn dropping_the_answer_hangs_up_on_the_provider() {
    let vendor = Vendor::open().await;
    let mut opening = vendor::message_events(&PIECES, 1_000, 100);
    opening.truncate(4);
    vendor.model("talking-model", Reply::Stalls(opening));
    let config = http_provider(
        "talker",
        "anthropic_compatible",
        &vendor.url(MESSAGES),
        "talking-model",
        "input_usd_per_mtok = 2.0",
    ) + &ladder("chat", &["talker"], 500);
    let bench = bench(&config, None);

    let mut events = bench
        .ladders
        .get("chat")
        .unwrap()
        .stream(request())
        .await
        .unwrap();
    assert_eq!(
        events.next().await,
        Some(Ok(Event::Delta("{\"say\":".to_owned())))
    );
    drop(events);

    assert!(
        vendor.hung_up(1).await,
        "the provider call is closed on the wire"
    );
    assert_eq!(
        bench.seen.steps(),
        [step("talker", "cancelled", (1_000, 1), 2_000)]
    );
}

#[tokio::test]
async fn a_stream_that_stalls_past_the_step_timeout_fails_the_request() {
    let vendor = Vendor::open().await;
    let mut opening = vendor::completion_chunks(&PIECES, None);
    opening.truncate(2);
    vendor.model("stalling-model", Reply::Stalls(opening));
    let config = http_provider(
        "staller",
        "openai_compatible",
        &vendor.url(COMPLETIONS),
        "stalling-model",
        "",
    )
    .replace("total_timeout_ms = 4000", "total_timeout_ms = 600")
        + &ladder("chat", &["staller"], 500);
    let bench = bench(&config, None);

    let answer = answer(&bench.ladders.get("chat").unwrap()).await;

    assert_eq!(answer.outcome, Err(LadderError::ProviderFailed));
    assert!(
        answer.finished >= Duration::from_millis(600) && answer.finished < Duration::from_secs(4)
    );
    assert_eq!(
        bench.seen.results(),
        results(&[("staller", "failed_timeout")])
    );
    assert!(vendor.hung_up(1).await);
}

#[tokio::test]
async fn a_provider_whose_secret_is_unset_is_left_out_of_its_ladders() {
    let vendor = Vendor::open().await;
    vendor.model(
        "good-model",
        Reply::Events(vendor::completion_chunks(&PIECES, None)),
    );
    let config = http_provider(
        "keyless",
        "anthropic_compatible",
        &vendor.url(MESSAGES),
        "good-model",
        "",
    )
    .replace("env:CARGO_PKG_NAME", "env:LLM_LADDER_TEST_UNSET_KEY")
        + &http_provider(
            "keyed",
            "openai_compatible",
            &vendor.url(COMPLETIONS),
            "good-model",
            "",
        )
        + &ladder("mixed", &["keyless", "keyed"], 500)
        + &ladder("empty", &["keyless"], 500);
    let bench = bench(&config, None);

    let answer = answer(&bench.ladders.get("mixed").unwrap()).await;
    assert_eq!(answer.outcome.unwrap().provider, "keyed");
    assert_eq!(bench.seen.results(), results(&[("keyed", "won")]));
    assert_eq!(vendor.calls().len(), 1);

    let unavailable = bench.ladders.get("empty").unwrap().stream(request()).await;
    assert_eq!(unavailable.err(), Some(LadderError::Unavailable));
    assert!(bench.ladders.get("unknown").is_none());
}

#[tokio::test]
async fn the_worker_kind_sends_the_contract_payload_and_reads_the_reply() {
    let local = Arc::new(LocalModel::default());
    *local.reply.lock().unwrap() = Some(Ok(json!({
        "text": format!("<think>short</think>{ANSWER}"),
        "model": "local-example",
        "input_tokens": 40,
        "output_tokens": 9,
    })));
    let config = "[providers.local]\nkind = \"worker\"\nsubject = \"ai.rpc.complete\"\nmax_output_tokens = 400\ntotal_timeout_ms = 5000\n"
        .to_owned()
        + &ladder("chat", &["local"], 500);
    let bench = bench(&config, Some(local.clone()));

    let answer = answer(&bench.ladders.get("chat").unwrap()).await;

    assert_eq!((answer.document.as_str(), answer.deltas), (ANSWER, 1));
    assert_eq!(
        answer.outcome.unwrap().usage,
        Usage {
            input_tokens: 40,
            output_tokens: 9,
            ..Usage::default()
        }
    );
    let calls = local.calls.lock().unwrap();
    let (subject, payload, timeout) = &calls[0];
    assert_eq!(subject, "ai.rpc.complete");
    assert_eq!(*timeout, Duration::from_secs(5));
    assert_eq!(
        *payload,
        json!({
            "system": "Answer with one JSON object.\n\nSpeak as Loli.\n\nThe volume is 60.",
            "messages": [
                {"role": "user", "content": "hi"},
                {"role": "assistant", "content": "hello"},
                {"role": "user", "content": "how are you"},
            ],
            "schema": {
                "type": "object",
                "properties": {
                    "say": {"type": "string"},
                    "mood": {"type": "string", "enum": ["calm", "glad"]},
                },
                "required": ["say", "mood"],
                "additionalProperties": false,
            },
            "max_tokens": 400,
        })
    );
}

#[tokio::test]
async fn a_worker_that_does_not_answer_gives_way_and_no_transport_turns_the_step_off() {
    let vendor = Vendor::open().await;
    vendor.model(
        "good-model",
        Reply::Events(vendor::completion_chunks(&PIECES, None)),
    );
    let local = Arc::new(LocalModel::default());
    *local.reply.lock().unwrap() = Some(Err(WorkerError::Timeout));
    let config = "[providers.local]\nkind = \"worker\"\nsubject = \"ai.rpc.complete\"\nmax_output_tokens = 400\ntotal_timeout_ms = 5000\n"
        .to_owned()
        + &http_provider("proxy", "openai_compatible", &vendor.url(COMPLETIONS), "good-model", "")
        + &ladder("chat", &["local", "proxy"], 3_000);

    let with_worker = bench(&config, Some(local.clone()));
    let answer_after_timeout = answer(&with_worker.ladders.get("chat").unwrap()).await;
    assert_eq!(answer_after_timeout.outcome.unwrap().provider, "proxy");
    assert_eq!(
        with_worker.seen.results(),
        results(&[("local", "failed_timeout"), ("proxy", "won")])
    );

    let without_worker = bench(&config, None);
    let answer_without = answer(&without_worker.ladders.get("chat").unwrap()).await;
    assert_eq!(answer_without.outcome.unwrap().provider, "proxy");
    assert_eq!(without_worker.seen.results(), results(&[("proxy", "won")]));
    assert_eq!(local.calls.lock().unwrap().len(), 1);
}
