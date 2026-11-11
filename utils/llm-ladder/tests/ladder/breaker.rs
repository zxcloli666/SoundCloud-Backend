use std::sync::Arc;
use std::time::Duration;

use crate::ANSWER;
use crate::bench::{Bench, PIECES, WAIT, answer, bench, http_provider, ladder};
use crate::vendor::{self, COMPLETIONS, MESSAGES, Reply, Vendor};
use llm_ladder::LadderError;
use serde_json::{Value, json};
use tokio::task::JoinSet;
use tokio::time::timeout;

const PROBES: usize = 8;

fn opening() -> Value {
    json!({"input_tokens": 900, "output_tokens": 1})
}

fn refusal() -> Reply {
    Reply::Events(vendor::message_events_ending(&[], opening(), 3, "refusal"))
}

fn cut() -> Reply {
    let long = ["{\"say\":\"once upon a time there was a very long"];
    Reply::Events(vendor::message_events_ending(
        &long,
        opening(),
        600,
        "max_tokens",
    ))
}

fn too_long() -> Reply {
    Reply::Status(
        400,
        json!({"type": "error", "error": {"type": "invalid_request_error", "message": "prompt is too long"}}),
    )
}

fn cut_completion() -> Reply {
    let mut chunks = vendor::completion_chunks(&["{\"say\":\"once upon a time"], Some((900, 600)));
    let finish = chunks.len() - 3;
    chunks[finish].1 = chunks[finish].1.replace("\"stop\"", "\"length\"");
    Reply::Events(chunks)
}

fn without_its_end(mut events: Vec<(u64, String)>, kept: usize) -> Reply {
    events.truncate(kept);
    Reply::Events(events)
}

async fn asked(bench: &Bench, ladder: &str) -> Result<String, LadderError> {
    let ladder = bench.ladders.get(ladder).expect("the ladder exists");
    answer(&ladder)
        .await
        .outcome
        .map(|outcome| outcome.provider)
}

#[tokio::test]
async fn refusals_cut_answers_and_requests_the_provider_cannot_take_never_open_the_breaker() {
    let vendor = Vendor::open().await;
    let config = http_provider(
        "asked",
        "anthropic_compatible",
        &vendor.url(MESSAGES),
        "asked-model",
        "",
    ) + &http_provider(
        "chatty",
        "openai_compatible",
        &vendor.url(COMPLETIONS),
        "chatty-model",
        "",
    ) + &ladder("chat", &["asked"], 500)
        + &ladder("talk", &["chatty"], 500);
    let bench = bench(&config, None);

    for (reply, failed) in [
        (refusal(), "failed_refused"),
        (cut(), "failed_length"),
        (too_long(), "failed_request"),
    ] {
        vendor.model("asked-model", reply);
        for _ in 0..3 {
            assert!(asked(&bench, "chat").await.is_err());
        }
        assert_eq!(bench.seen.results().last().unwrap().1, failed);
    }
    vendor.model("chatty-model", cut_completion());
    for _ in 0..3 {
        assert!(asked(&bench, "talk").await.is_err());
    }
    assert_eq!(bench.seen.results().last().unwrap().1, "failed_length");

    vendor.model(
        "asked-model",
        Reply::Events(vendor::message_events(&PIECES, 900, 20)),
    );
    vendor.model(
        "chatty-model",
        Reply::Events(vendor::completion_chunks(&PIECES, None)),
    );
    assert_eq!(asked(&bench, "chat").await.as_deref(), Ok("asked"));
    assert_eq!(asked(&bench, "talk").await.as_deref(), Ok("chatty"));
    assert_eq!(vendor.calls_of("asked-model").len(), 10);
    assert_eq!(vendor.calls_of("chatty-model").len(), 4);
    assert!(
        !bench
            .seen
            .results()
            .iter()
            .any(|(_, result)| *result == "skipped_breaker")
    );
}

#[tokio::test]
async fn a_stream_that_closes_before_its_answer_ends_counts_against_the_provider() {
    let vendor = Vendor::open().await;
    let config = http_provider(
        "asked",
        "anthropic_compatible",
        &vendor.url(MESSAGES),
        "asked-model",
        "",
    ) + &http_provider(
        "chatty",
        "openai_compatible",
        &vendor.url(COMPLETIONS),
        "chatty-model",
        "",
    ) + &ladder("chat", &["asked"], 500)
        + &ladder("talk", &["chatty"], 500);
    let bench = bench(&config, None);
    vendor.model(
        "asked-model",
        without_its_end(vendor::message_events(&PIECES, 900, 20), 5),
    );
    vendor.model(
        "chatty-model",
        without_its_end(vendor::completion_chunks(&PIECES, None), 3),
    );

    for ladder in ["chat", "talk"] {
        for _ in 0..3 {
            assert_eq!(
                asked(&bench, ladder).await,
                Err(LadderError::ProviderFailed)
            );
        }
        assert_eq!(bench.seen.results().last().unwrap().1, "failed_transport");
        assert_eq!(asked(&bench, ladder).await, Err(LadderError::Unavailable));
    }
    assert_eq!(vendor.calls().len(), 6);
}

#[tokio::test]
async fn an_answer_whose_end_was_announced_stands_without_the_last_line_of_its_stream() {
    let vendor = Vendor::open().await;
    let config = http_provider(
        "asked",
        "anthropic_compatible",
        &vendor.url(MESSAGES),
        "asked-model",
        "",
    ) + &http_provider(
        "chatty",
        "openai_compatible",
        &vendor.url(COMPLETIONS),
        "chatty-model",
        "",
    ) + &ladder("chat", &["asked"], 500)
        + &ladder("talk", &["chatty"], 500);
    let bench = bench(&config, None);
    let messages = vendor::message_events(&PIECES, 900, 20);
    let stopped = messages.len() - 1;
    vendor.model("asked-model", without_its_end(messages, stopped));
    let chunks = vendor::completion_chunks(&PIECES, None);
    let finished = chunks.len() - 1;
    assert_eq!(chunks[finished].1, vendor::DONE);
    vendor.model("chatty-model", without_its_end(chunks, finished));

    assert_eq!(asked(&bench, "chat").await.as_deref(), Ok("asked"));
    assert_eq!(asked(&bench, "talk").await.as_deref(), Ok("chatty"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_half_open_breaker_lets_exactly_one_of_many_concurrent_requests_probe() {
    let vendor = Vendor::open().await;
    let config = http_provider(
        "flaky",
        "anthropic_compatible",
        &vendor.url(MESSAGES),
        "flaky-model",
        "stream = false",
    ) + &http_provider(
        "spare",
        "openai_compatible",
        &vendor.url(COMPLETIONS),
        "spare-model",
        "",
    ) + &ladder("chat", &["flaky", "spare"], 7_000)
        .replace("open_s = 90", "open_s = 1");
    let bench = Arc::new(bench(&config, None));
    vendor.model(
        "flaky-model",
        Reply::Status(
            529,
            json!({"type": "error", "error": {"type": "overloaded_error"}}),
        ),
    );
    vendor.model(
        "spare-model",
        Reply::Events(vendor::completion_chunks(&PIECES, None)),
    );
    for _ in 0..3 {
        assert_eq!(asked(&bench, "chat").await.as_deref(), Ok("spare"));
    }
    assert_eq!(asked(&bench, "chat").await.as_deref(), Ok("spare"));
    assert_eq!(
        vendor.calls_of("flaky-model").len(),
        3,
        "the breaker is open"
    );

    tokio::time::sleep(Duration::from_millis(1_100)).await;
    vendor.model(
        "flaky-model",
        Reply::GatedWhole(vendor::whole_message(ANSWER, 900, 20)),
    );
    let mut asking = JoinSet::new();
    for _ in 0..PROBES {
        let bench = bench.clone();
        asking.spawn(async move { asked(&bench, "chat").await });
    }
    for _ in 1..PROBES {
        let answered = timeout(WAIT, asking.join_next())
            .await
            .expect("the requests that skip the probe are answered")
            .expect("a request is left")
            .unwrap();
        assert_eq!(answered.as_deref(), Ok("spare"));
    }
    assert_eq!(
        vendor.calls_of("flaky-model").len(),
        4,
        "only one of {PROBES} concurrent requests reached the half open provider"
    );

    vendor.release();
    let probe = asking
        .join_next()
        .await
        .expect("the probe is left")
        .unwrap();
    assert_eq!(probe.as_deref(), Ok("flaky"));
    vendor.model(
        "flaky-model",
        Reply::Whole(0, vendor::whole_message(ANSWER, 900, 20)),
    );
    assert_eq!(asked(&bench, "chat").await.as_deref(), Ok("flaky"));
    assert_eq!(vendor.calls_of("flaky-model").len(), 5);
}
