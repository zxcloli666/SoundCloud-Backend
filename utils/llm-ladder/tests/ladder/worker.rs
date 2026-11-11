use std::sync::Arc;
use std::time::Duration;

use crate::ANSWER;
use crate::bench::{
    LocalModel, PIECES, Said, answer, answer_to, bench, http_provider, ladder, request, results,
    worker_provider,
};
use crate::vendor::{self, COMPLETIONS, Reply, Vendor};
use bytes::Bytes;
use futures::future::{BoxFuture, pending};
use llm_ladder::{LadderError, Request, System, Usage, WorkerError, WorkerTransport};
use serde_json::{Value, json};

struct Thinking;

impl WorkerTransport for Thinking {
    fn call<'a>(
        &'a self,
        _subject: &'a str,
        _payload: Bytes,
        _timeout: Duration,
    ) -> BoxFuture<'a, Result<Value, WorkerError>> {
        Box::pin(pending())
    }
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
    let config = worker_provider("local", "") + &ladder("chat", &["local"], 500);
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
async fn a_request_too_large_for_the_worker_is_never_published() {
    let local = Arc::new(LocalModel::default());
    *local.reply.lock().unwrap() = Some(Ok(json!({"text": ANSWER})));
    let config = worker_provider("local", "") + &ladder("chat", &["local"], 500);
    let bench = bench(&config, Some(local.clone()));
    let large = Request {
        system: System::fixed("rules ".repeat(8_001)),
        ..request()
    };

    let refused = bench
        .ladders
        .get("chat")
        .unwrap()
        .complete::<Said>(large)
        .await;

    assert_eq!(refused.err(), Some(LadderError::Exhausted));
    assert_eq!(
        bench.seen.results(),
        results(&[("local", "failed_request")])
    );
    assert!(local.calls.lock().unwrap().is_empty());
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
    let config = worker_provider("local", "")
        + &http_provider(
            "proxy",
            "openai_compatible",
            &vendor.url(COMPLETIONS),
            "good-model",
            "",
        )
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

#[tokio::test]
async fn a_worker_out_of_the_hedge_starts_after_the_silent_step_and_hears_the_time_left() {
    let vendor = Vendor::open().await;
    let mut opening = vendor::completion_chunks(&PIECES, None);
    opening.truncate(1);
    vendor.model("silent-model", Reply::Stalls(opening));
    let local = Arc::new(LocalModel::default());
    *local.reply.lock().unwrap() = Some(Ok(json!({"text": ANSWER, "model": "local-example"})));
    let config = http_provider(
        "silent",
        "openai_compatible",
        &vendor.url(COMPLETIONS),
        "silent-model",
        "",
    )
    .replace("total_timeout_ms = 4000", "total_timeout_ms = 800")
        + &worker_provider("local", "hedge = false")
            .replace("total_timeout_ms = 5000", "total_timeout_ms = 1900")
        + &ladder("chat", &["silent", "local"], 100)
            .replace("deadline_ms = 8000", "deadline_ms = 2000");
    let bench = bench(&config, Some(local.clone()));

    let answer = answer(&bench.ladders.get("chat").unwrap()).await;

    assert_eq!(answer.outcome.unwrap().provider, "local");
    assert_eq!(
        bench.seen.results(),
        results(&[("silent", "failed_timeout"), ("local", "won")])
    );
    let calls = local.calls.lock().unwrap();
    let (_, _, time_left) = &calls[0];
    assert!(
        *time_left <= Duration::from_millis(1_200),
        "the worker started after the silent step and was given what was left of the 2 s: {time_left:?}"
    );
}

#[tokio::test]
async fn a_worker_cut_by_the_deadline_is_not_blamed_for_its_silence() {
    let config = worker_provider("local", "") + &ladder("chat", &["local"], 100);
    let bench = bench(&config, Some(Arc::new(Thinking)));
    let short = Request {
        deadline: Some(Duration::from_millis(600)),
        ..request()
    };

    let answer = answer_to(&bench.ladders.get("chat").unwrap(), short).await;

    assert_eq!(answer.outcome, Err(LadderError::Deadline));
    assert_eq!(bench.seen.results(), results(&[("local", "cancelled")]));
}

#[tokio::test]
async fn a_request_the_worker_calls_invalid_does_not_count_against_it() {
    let local = Arc::new(LocalModel::default());
    *local.reply.lock().unwrap() = Some(Err(WorkerError::Invalid));
    let config = worker_provider("local", "") + &ladder("chat", &["local"], 500);
    let bench = bench(&config, Some(local.clone()));

    for _ in 0..5 {
        let answer = answer(&bench.ladders.get("chat").unwrap()).await;
        assert_eq!(answer.outcome, Err(LadderError::Exhausted));
    }

    assert_eq!(local.calls.lock().unwrap().len(), 5);
    assert!(
        bench
            .seen
            .results()
            .iter()
            .all(|(_, result)| *result == "failed_request")
    );
}
