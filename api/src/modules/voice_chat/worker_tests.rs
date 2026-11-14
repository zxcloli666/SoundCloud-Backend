use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use backend_contracts::pipeline::{
    AI_COMPLETE, CompleteRequest, DEADLINE_HEADER, MAX_COMPLETE_MESSAGE_BYTES,
    MAX_COMPLETE_MESSAGES, MAX_COMPLETE_SCHEMA_BYTES, MAX_COMPLETE_SYSTEM_BYTES,
    MAX_COMPLETE_SYSTEM_MARKS, MAX_COMPLETE_TOKENS, MSG_ID_HEADER, REPLY_TO_HEADER,
};
use bytes::Bytes;
use futures::future::BoxFuture;
use llm_ladder::{
    Hosting, Observer, Section, StepReport, WORKER_LIMITS, WorkerError, WorkerLimits,
    WorkerTransport,
};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::config::LlmCfg;
use crate::modules::voice_chat::VoiceChat;
use crate::modules::voice_chat::request::ChatBody;
use crate::modules::voice_chat::testing::{CHAT_SECTION, Heard, hear, lane, tools};

const NOTE: &str = "зови меня Саша";
const SPEECH: &str = "{\"actions\":[],\"say\":\"Привет.\"}";
const LOUDER: &str = "{\"actions\":[{\"name\":\"volume_set\",\"args\":[{\"name\":\"level\",\"value\":90}]}],\"say\":\"Громче.\"}";
const NOTHING: &str = "{\"actions\":[],\"say\":\"\"}";
const NO_WORD: &str = "{\"say\":\" \"}";
const PLAIN: &str = "plain = true\n";

struct Asked {
    answer: &'static str,
    payloads: Mutex<Vec<Bytes>>,
}

struct Unwatched;

struct Talk {
    heard: Heard,
    sections: Vec<Section>,
    sent: Value,
}

impl WorkerTransport for Asked {
    fn call<'a>(
        &'a self,
        _: &'a str,
        payload: Bytes,
        _: Duration,
    ) -> BoxFuture<'a, Result<Value, WorkerError>> {
        self.payloads.lock().unwrap().push(payload);
        let data = json!({"text": self.answer, "input_tokens": 40, "output_tokens": 9});
        Box::pin(async move { Ok(data) })
    }
}

impl Observer for Unwatched {
    fn step(&self, _: &StepReport<'_>) {}
}

async fn talk(answer: &'static str, provider_keys: &str) -> Talk {
    told(phrase(), answer, provider_keys).await
}

async fn overheard(answer: &'static str, provider_keys: &str) -> Talk {
    let mut body = phrase();
    body["quiet"] = json!(true);
    told(body, answer, provider_keys).await
}

fn phrase() -> Value {
    json!({
        "text": "привет",
        "persona": "loli",
        "language": "ru",
        "tools": tools(),
        "notes": [NOTE],
        "context": {"volume": 40},
    })
}

async fn told(body: Value, answer: &'static str, provider_keys: &str) -> Talk {
    let asked = Arc::new(Asked {
        answer,
        payloads: Mutex::default(),
    });
    let hosting = Hosting {
        jetstream: None,
        worker: Some(asked.clone()),
        observer: Arc::new(Unwatched),
    };
    let chat = VoiceChat::hosted(&local_only(provider_keys), hosting).expect("the chat ladder");
    let live = chat.live().expect("a live chat");
    let body: ChatBody = serde_json::from_value(body).expect("a chat body");
    let turn = body.into_turn(live.limits()).expect("a valid turn");
    let request = live
        .request(&turn, "contract")
        .expect("a prompt within the cap");
    let sections = request.system.sections.clone();
    let response = live.answer(request, turn).await.expect("an open step");
    let heard = hear(response).await.expect("a stream");
    let payloads = asked.payloads.lock().unwrap();
    Talk {
        heard,
        sections,
        sent: serde_json::from_slice(&payloads[0]).expect("a JSON request"),
    }
}

fn local_only(provider_keys: &str) -> LlmCfg {
    let ladder = format!(
        "[providers.local]\nkind = \"worker\"\nsubject = \"{AI_COMPLETE}\"\nmax_output_tokens = 400\ntotal_timeout_ms = 5000\n{provider_keys}\n[ladders.chat]\nsteps = [\"local\"]\nhedge_after_ms = 2000\ndeadline_ms = 6000\nbreaker = {{ failures = 3, window_s = 30, open_s = 90 }}\n"
    );
    LlmCfg::parse(&format!("{ladder}{CHAT_SECTION}")).expect("a valid config")
}

fn of_the_contract(sent: &Value) -> CompleteRequest {
    let read: CompleteRequest =
        serde_json::from_value(sent.clone()).expect("the request of the contract");
    assert_eq!(serde_json::to_value(&read).expect("a JSON request"), *sent);
    read
}

fn events(heard: &Heard) -> Vec<&str> {
    heard.named().into_iter().map(|(event, _)| event).collect()
}

fn sized(limit: u32) -> usize {
    usize::try_from(limit).expect("a limit that fits the platform")
}

#[tokio::test]
async fn a_chat_turn_reaches_the_worker_as_the_request_of_the_contract() {
    let talk = talk(SPEECH, "").await;

    assert_eq!(
        talk.heard.named(),
        [("say", &json!({"text": "Привет."})), ("done", &json!({}))]
    );
    let read = of_the_contract(&talk.sent);
    let rules = talk.sections[0].text.len();
    let character = rules + "\n\n".len() + talk.sections[1].text.len();
    let marks: Vec<usize> = read.system_marks.iter().copied().map(sized).collect();
    assert_eq!(marks, [rules, character]);
    assert_eq!(read.system[..rules], talk.sections[0].text);
    assert!(read.system[..character].ends_with(&talk.sections[1].text));
    assert!(!read.system[..character].contains(NOTE));
    assert!(read.system[character..].contains(NOTE));
    assert!(read.system.contains("volume_set"));
    assert_eq!(talk.sent["schema"]["required"], json!(["actions", "say"]));
}

#[tokio::test]
async fn a_plain_worker_is_asked_the_turn_without_tools() {
    let talk = talk(SPEECH, PLAIN).await;

    assert_eq!(
        talk.heard.named(),
        [("say", &json!({"text": "Привет."})), ("done", &json!({}))]
    );
    let read = of_the_contract(&talk.sent);
    assert_eq!(read.system_marks.len(), 2);
    assert!(read.system.contains("No tools are available"));
    assert!(!read.system.contains("Tools:"));
    assert!(!read.system.contains("volume_set"));
    assert!(read.system.contains("A cheerful voice named Loli."));
    assert!(read.system.contains(NOTE));
    assert_eq!(talk.sent["schema"]["required"], json!(["say"]));
    assert_eq!(
        talk.sent["schema"]["properties"]
            .as_object()
            .map(|keys| keys.len()),
        Some(1)
    );
}

#[tokio::test]
async fn an_action_is_run_for_a_trusted_worker_and_refused_from_a_plain_one() {
    let trusted = talk(LOUDER, "").await;
    assert_eq!(events(&trusted.heard), ["action", "say", "done"]);

    let plain = talk(LOUDER, PLAIN).await;
    assert_eq!(
        plain.heard.named(),
        [("error", &json!({"code": "provider_failed"}))]
    );
}

#[tokio::test]
async fn an_answer_that_does_and_says_nothing_is_a_silence_for_an_overheard_phrase() {
    for nothing in [NOTHING, NO_WORD] {
        let silent = overheard(nothing, "").await;
        assert_eq!(
            silent.heard.named(),
            [("quiet", &json!({})), ("done", &json!({}))]
        );
        assert!(
            of_the_contract(&silent.sent)
                .system
                .contains("Staying silent.")
        );

        let asked = talk(nothing, "").await;
        assert_eq!(
            asked.heard.named(),
            [("error", &json!({"code": "provider_failed"}))]
        );
        assert!(
            !of_the_contract(&asked.sent)
                .system
                .contains("Staying silent.")
        );
    }
    let answered = overheard(LOUDER, "").await;
    assert_eq!(events(&answered.heard), ["action", "say", "done"]);
}

#[tokio::test]
async fn a_plain_worker_is_not_let_to_stay_silent() {
    let mut with_tools = phrase();
    with_tools["quiet"] = json!(true);
    let mut with_no_tools = with_tools.clone();
    with_no_tools["tools"] = json!([]);
    for body in [with_tools, with_no_tools] {
        let plain = told(body, NO_WORD, PLAIN).await;
        assert_eq!(
            plain.heard.named(),
            [("error", &json!({"code": "provider_failed"}))]
        );
        assert!(
            !of_the_contract(&plain.sent)
                .system
                .contains("Staying silent.")
        );
    }
    let spoken = overheard(SPEECH, PLAIN).await;
    assert_eq!(events(&spoken.heard), ["say", "done"]);
}

#[test]
fn the_ladder_measures_a_worker_request_by_the_limits_of_the_contract() {
    assert_eq!(
        WORKER_LIMITS,
        WorkerLimits {
            max_tokens: MAX_COMPLETE_TOKENS,
            system_bytes: sized(MAX_COMPLETE_SYSTEM_BYTES),
            system_marks: sized(MAX_COMPLETE_SYSTEM_MARKS),
            messages: sized(MAX_COMPLETE_MESSAGES),
            message_bytes: sized(MAX_COMPLETE_MESSAGE_BYTES),
            schema_bytes: sized(MAX_COMPLETE_SCHEMA_BYTES),
        }
    );
}

#[tokio::test]
#[ignore = "requires a local NATS with JetStream"]
async fn a_call_goes_out_with_the_rpc_headers_and_the_data_comes_back() {
    let lane = lane().await;
    let reply = json!({"ok": true, "data": {"text": "{\"say\":\"hi\"}", "input_tokens": 40, "output_tokens": 9}});
    let worker = lane.worker(Some(reply)).await;
    let before = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis();

    let payload = Bytes::from_static(br#"{"system":"rules","max_tokens":400}"#);
    let data = lane
        .rpc
        .call(&lane.subject, payload.clone(), Duration::from_secs(5))
        .await
        .expect("the worker answers");
    let after = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis();

    assert_eq!(
        data,
        json!({"text": "{\"say\":\"hi\"}", "input_tokens": 40, "output_tokens": 9})
    );
    let call = worker.await.unwrap();
    assert_eq!(call.payload, payload);
    let headers = call.headers.expect("headers");
    assert!(
        headers
            .get(REPLY_TO_HEADER)
            .unwrap()
            .as_str()
            .starts_with("_INBOX.")
    );
    assert!(Uuid::parse_str(headers.get(MSG_ID_HEADER).unwrap().as_str()).is_ok());
    let expires: u128 = headers
        .get(DEADLINE_HEADER)
        .unwrap()
        .as_str()
        .parse()
        .unwrap();
    assert!(
        (before + 5_000..=after + 5_000).contains(&expires),
        "{expires} is five seconds after the call, made between {before} and {after}"
    );
    lane.close().await;
}

#[tokio::test]
#[ignore = "requires a local NATS with JetStream"]
async fn a_refusal_a_silence_and_a_missing_stream_are_told_apart() {
    let lane = lane().await;
    let refusing = lane
        .worker(Some(json!({"ok": false, "error": "model not loaded"})))
        .await;
    let refused = lane
        .rpc
        .call(&lane.subject, Bytes::new(), Duration::from_secs(5))
        .await;
    assert_eq!(refused, Err(WorkerError::Rejected));
    refusing.await.unwrap();

    let invalid = lane
        .worker(Some(json!({"ok": false, "error": "invalid_request"})))
        .await;
    let too_large = lane
        .rpc
        .call(&lane.subject, Bytes::new(), Duration::from_secs(5))
        .await;
    assert_eq!(too_large, Err(WorkerError::Invalid));
    invalid.await.unwrap();

    let silent = lane.worker(None).await;
    let unanswered = lane
        .rpc
        .call(&lane.subject, Bytes::new(), Duration::from_millis(300))
        .await;
    assert_eq!(unanswered, Err(WorkerError::Timeout));
    silent.await.unwrap();

    let nowhere = lane
        .rpc
        .call(
            "llmtest.nostream.complete",
            Bytes::new(),
            Duration::from_secs(2),
        )
        .await;
    assert_eq!(nowhere, Err(WorkerError::Unreachable));
    lane.close().await;
}
