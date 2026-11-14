use std::time::Duration;

use super::*;

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
