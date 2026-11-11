use std::time::Duration;

use serde_json::json;
use uuid::Uuid;

use super::*;
use crate::modules::voice_chat::testing::lane;

const SYSTEM_TIME_SLACK_MS: u128 = 2_000;

#[test]
fn the_deadline_is_the_unix_millisecond_the_call_expires_at() {
    let at = UNIX_EPOCH + Duration::from_millis(1_700_000_000_250);
    assert_eq!(deadline(at, Duration::from_secs(30)), "1700000030250");
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
        (before + 5_000..=before + 5_000 + SYSTEM_TIME_SLACK_MS).contains(&expires),
        "{expires} is five seconds after {before}"
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
