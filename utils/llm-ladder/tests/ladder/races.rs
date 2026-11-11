use std::time::Duration;

use crate::ANSWER;
use crate::bench::{
    PIECES, answer, answer_to, any_text, bench, http_provider, ladder, request, results, step,
};
use crate::vendor::{self, COMPLETIONS, MESSAGES, Reply, Step, Vendor};
use futures::StreamExt;
use llm_ladder::{Event, LadderError, Request};
use serde_json::json;

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
    ) + &ladder("chat", &["down", "prose", "good"], 7_000);
    let bench = bench(&config, None);

    let answer = answer(&bench.ladders.get("chat").unwrap()).await;

    assert_eq!(answer.document, ANSWER);
    assert_eq!(
        answer.outcome.unwrap().provider,
        "good",
        "a failure starts the next step at once: two waits for the hedge would pass the deadline"
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
        Reply::GatedWhole(vendor::whole_message(ANSWER, 1, 1)),
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
async fn a_one_shot_step_cut_by_the_deadline_is_not_blamed_for_its_silence() {
    let vendor = Vendor::open().await;
    vendor.model(
        "whole-model",
        Reply::GatedWhole(vendor::whole_message(ANSWER, 1, 1)),
    );
    let config = http_provider(
        "whole",
        "anthropic_compatible",
        &vendor.url(MESSAGES),
        "whole-model",
        "stream = false",
    ) + &ladder("chat", &["whole"], 100);
    let bench = bench(&config, None);
    let short = Request {
        deadline: Some(Duration::from_millis(600)),
        ..request()
    };

    let answer = answer_to(&bench.ladders.get("chat").unwrap(), short).await;

    assert_eq!(answer.outcome, Err(LadderError::Deadline));
    assert_eq!(bench.seen.results(), results(&[("whole", "cancelled")]));
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
        "input_usd_per_mtok = 2.0\noutput_usd_per_mtok = 8.0",
    ) + &ladder("chat", &["talker"], 500);
    let bench = bench(&config, None);

    let mut events = bench
        .ladders
        .get("chat")
        .unwrap()
        .stream(request(), any_text())
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
    let charged = Step {
        basis: "estimated",
        ..step("talker", "cancelled", (1_000, 3), 2_024)
    };
    assert_eq!(
        bench.seen.steps(),
        [charged],
        "the seven bytes it sent count as three tokens, not the one of its first event"
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

    assert_eq!(
        answer.outcome,
        Err(LadderError::ProviderFailed),
        "the step timeout ends it, not the deadline"
    );
    assert!(answer.finished >= Duration::from_millis(600));
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

    let unavailable = bench
        .ladders
        .get("empty")
        .unwrap()
        .stream(request(), any_text())
        .await;
    assert_eq!(unavailable.err(), Some(LadderError::Unavailable));
    assert!(bench.ladders.get("unknown").is_none());
}
