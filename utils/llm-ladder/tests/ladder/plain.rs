use std::sync::{Arc, Mutex};

use llm_ladder::{Commit, Form, Message, Plain, Request, Schema, System};
use serde_json::{Value, json};

use crate::ANSWER;
use crate::bench::{
    LocalModel, PIECES, answer_judged, bench, http_provider, ladder, request, results,
    worker_provider,
};
use crate::vendor::{self, COMPLETIONS, Reply, Vendor};

const FULL_RULES: &str = "Answer with one JSON object.\n\nSpeak as Loli.\n\nThe volume is 60.";
const PLAIN_RULES: &str = "Answer in one sentence.";

type Forms = Arc<Mutex<Vec<Form>>>;

fn asked_both_ways() -> Request {
    Request {
        plain: Some(Plain {
            system: System::fixed(PLAIN_RULES),
            messages: vec![Message::user("how are you")],
            schema: Schema::object([("say", Schema::String)]),
        }),
        ..request()
    }
}

fn answering() -> Arc<LocalModel> {
    let local = Arc::new(LocalModel::default());
    *local.reply.lock().unwrap() = Some(Ok(
        json!({"text": ANSWER, "input_tokens": 40, "output_tokens": 9}),
    ));
    local
}

fn noting(forms: &Forms) -> Commit {
    let forms = forms.clone();
    Arc::new(move |form, _| {
        forms.lock().unwrap().push(form);
        true
    })
}

fn only_full() -> Commit {
    Arc::new(|form, _| form == Form::Full)
}

fn sent_to(local: &LocalModel) -> Value {
    local.calls.lock().unwrap()[0].1.clone()
}

#[tokio::test]
async fn a_plain_provider_is_asked_the_plain_form_and_judged_by_it() {
    let local = answering();
    let config = worker_provider("local", "plain = true") + &ladder("chat", &["local"], 500);
    let bench = bench(&config, Some(local.clone()));
    let forms = Forms::default();
    let chat = bench.ladders.get("chat").unwrap();

    let answer = answer_judged(&chat, asked_both_ways(), noting(&forms)).await;

    assert_eq!(answer.document, ANSWER);
    let sent = sent_to(&local);
    assert_eq!(sent["system"], PLAIN_RULES);
    assert_eq!(
        sent["messages"],
        json!([{"role": "user", "content": "how are you"}])
    );
    assert_eq!(sent["schema"]["required"], json!(["say"]));
    assert_eq!(*forms.lock().unwrap(), [Form::Plain]);
}

#[tokio::test]
async fn a_provider_not_marked_plain_is_asked_the_full_form() {
    let local = answering();
    let config = worker_provider("local", "") + &ladder("chat", &["local"], 500);
    let bench = bench(&config, Some(local.clone()));
    let forms = Forms::default();
    let chat = bench.ladders.get("chat").unwrap();

    answer_judged(&chat, asked_both_ways(), noting(&forms)).await;

    let sent = sent_to(&local);
    assert_eq!(sent["system"], FULL_RULES);
    assert_eq!(sent["messages"].as_array().map(Vec::len), Some(3));
    assert_eq!(sent["schema"]["required"], json!(["say", "mood"]));
    assert_eq!(*forms.lock().unwrap(), [Form::Full]);
}

#[tokio::test]
async fn a_request_with_no_plain_form_goes_whole_to_a_plain_provider() {
    let local = answering();
    let config = worker_provider("local", "plain = true") + &ladder("chat", &["local"], 500);
    let bench = bench(&config, Some(local.clone()));
    let forms = Forms::default();
    let chat = bench.ladders.get("chat").unwrap();

    answer_judged(&chat, request(), noting(&forms)).await;

    assert_eq!(sent_to(&local)["system"], FULL_RULES);
    assert_eq!(*forms.lock().unwrap(), [Form::Full]);
}

#[tokio::test]
async fn each_step_of_one_ladder_is_asked_its_own_form() {
    let vendor = Vendor::open().await;
    vendor.model(
        "full-model",
        Reply::Events(vendor::completion_chunks(&PIECES, None)),
    );
    let local = answering();
    let config = worker_provider("local", "plain = true")
        + &http_provider(
            "proxy",
            "openai_compatible",
            &vendor.url(COMPLETIONS),
            "full-model",
            "",
        )
        + &ladder("chat", &["local", "proxy"], 3_000);
    let bench = bench(&config, Some(local.clone()));
    let chat = bench.ladders.get("chat").unwrap();

    let answer = answer_judged(&chat, asked_both_ways(), only_full()).await;

    assert_eq!(answer.outcome.unwrap().provider, "proxy");
    assert_eq!(
        bench.seen.results(),
        results(&[("local", "failed_invalid"), ("proxy", "won")])
    );
    assert_eq!(sent_to(&local)["system"], PLAIN_RULES);
    let asked = vendor.calls_of("full-model")[0].json();
    assert_eq!(asked["messages"][0]["content"], FULL_RULES);
    assert_eq!(asked["messages"].as_array().map(Vec::len), Some(4));
}
