use std::sync::Mutex;
use std::time::Duration;

use backend_contracts::pipeline::{
    AI_COMPLETE, CompleteRequest, MAX_COMPLETE_MESSAGE_BYTES, MAX_COMPLETE_MESSAGES,
    MAX_COMPLETE_SCHEMA_BYTES, MAX_COMPLETE_SYSTEM_BYTES, MAX_COMPLETE_SYSTEM_MARKS,
    MAX_COMPLETE_TOKENS,
};
use llm_ladder::{Hosting, Observer, StepReport, WORKER_LIMITS, WorkerLimits};
use serde_json::json;
use uuid::Uuid;

use super::*;
use crate::config::LlmCfg;
use crate::modules::voice_chat::VoiceChat;
use crate::modules::voice_chat::request::ChatBody;
use crate::modules::voice_chat::testing::{CHAT_SECTION, hear, lane, tools};

const NOTE: &str = "зови меня Саша";

#[derive(Default)]
struct Asked {
    payloads: Mutex<Vec<Bytes>>,
}

struct Unwatched;

impl WorkerTransport for Asked {
    fn call<'a>(
        &'a self,
        _: &'a str,
        payload: Bytes,
        _: Duration,
    ) -> BoxFuture<'a, Result<Value, WorkerError>> {
        self.payloads.lock().unwrap().push(payload);
        let data = json!({"text": "{\"actions\":[],\"say\":\"Привет.\"}", "input_tokens": 40, "output_tokens": 9});
        Box::pin(async move { Ok(data) })
    }
}

impl Observer for Unwatched {
    fn step(&self, _: &StepReport<'_>) {}
}

fn local_only() -> LlmCfg {
    let ladder = format!(
        "[providers.local]\nkind = \"worker\"\nsubject = \"{AI_COMPLETE}\"\nmax_output_tokens = 400\ntotal_timeout_ms = 5000\n\n[ladders.chat]\nsteps = [\"local\"]\nhedge_after_ms = 2000\ndeadline_ms = 6000\nbreaker = {{ failures = 3, window_s = 30, open_s = 90 }}\n"
    );
    LlmCfg::parse(&format!("{ladder}{CHAT_SECTION}")).expect("a valid config")
}

fn sized(limit: u32) -> usize {
    usize::try_from(limit).expect("a limit that fits the platform")
}

#[tokio::test]
async fn a_chat_turn_reaches_the_worker_as_the_request_of_the_contract() {
    let asked = Arc::new(Asked::default());
    let hosting = Hosting {
        jetstream: None,
        worker: Some(asked.clone()),
        observer: Arc::new(Unwatched),
    };
    let chat = VoiceChat::hosted(&local_only(), hosting).expect("the chat ladder");
    let live = chat.live().expect("a live chat");
    let body: ChatBody = serde_json::from_value(json!({
        "text": "привет",
        "persona": "loli",
        "language": "ru",
        "tools": tools(),
        "notes": [NOTE],
        "context": {"volume": 40},
    }))
    .expect("a chat body");
    let turn = body.into_turn(live.limits()).expect("a valid turn");
    let request = live
        .request(&turn, "contract")
        .expect("a prompt within the cap");
    let sections = request.system.sections.clone();

    let response = live
        .answer(request, turn.tools)
        .await
        .expect("an open step");
    let heard = hear(response).await.expect("a stream");

    assert_eq!(
        heard.named(),
        [("say", &json!({"text": "Привет."})), ("done", &json!({}))]
    );
    let payloads = asked.payloads.lock().unwrap();
    let sent: Value = serde_json::from_slice(&payloads[0]).expect("a JSON request");
    let read: CompleteRequest =
        serde_json::from_value(sent.clone()).expect("the request of the contract");
    assert_eq!(serde_json::to_value(&read).expect("a JSON request"), sent);
    let rules = sections[0].text.len();
    let character = rules + "\n\n".len() + sections[1].text.len();
    let marks: Vec<usize> = read.system_marks.iter().copied().map(sized).collect();
    assert_eq!(marks, [rules, character]);
    assert_eq!(read.system[..rules], sections[0].text);
    assert!(read.system[..character].ends_with(&sections[1].text));
    assert!(!read.system[..character].contains(NOTE));
    assert!(read.system[character..].contains(NOTE));
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

#[test]
fn the_deadline_is_the_unix_millisecond_the_call_expires_at() {
    let at = UNIX_EPOCH + Duration::from_millis(1_700_000_000_250);
    assert_eq!(deadline(at, Duration::from_secs(30)), "1700000030250");
}

#[test]
fn a_refusal_of_the_request_is_told_apart_from_a_fault_of_the_worker() {
    assert_eq!(refusal("invalid_request"), WorkerError::Invalid);
    assert_eq!(refusal("expired"), WorkerError::Timeout);
    assert_eq!(refusal("internal"), WorkerError::Rejected);
    assert_eq!(refusal("model not loaded"), WorkerError::Rejected);
    assert_eq!(refusal(""), WorkerError::Rejected);
}

#[test]
fn only_a_short_label_of_a_worker_error_is_logged() {
    assert_eq!(
        reason("model not loaded: slot llm-local"),
        "model_not_loaded_slot_llm-local"
    );
    assert_eq!(reason("Ошибка: «текст фразы» \"quoted\""), "___quoted");
    assert_eq!(reason(&"x".repeat(200)).len(), 48);
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
