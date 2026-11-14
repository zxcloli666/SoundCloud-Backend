use axum::body::to_bytes;
use serde_json::Value;

use super::*;
use crate::modules::voice_speak::testing::{CLIP, REVISION};

async fn body(response: Response) -> Bytes {
    to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("a body")
}

fn content_type(response: &Response) -> Option<&str> {
    header(response, CONTENT_TYPE.as_str())
}

fn revision(response: &Response) -> Option<&str> {
    header(response, REVISION_HEADER)
}

fn header<'a>(response: &'a Response, name: &str) -> Option<&'a str> {
    response
        .headers()
        .get(name)
        .and_then(|value| value.to_str().ok())
}

#[tokio::test]
async fn a_cached_line_is_an_ogg_clip_and_a_new_one_is_a_ticket_both_with_the_revision() {
    let ticket = Uuid::now_v7();

    let cached = answer(Said::Cached(Bytes::from_static(CLIP)), REVISION);
    let asked = answer(Said::Ticket(ticket), REVISION);

    assert_eq!(cached.status(), StatusCode::OK);
    assert_eq!(content_type(&cached), Some("audio/ogg"));
    assert_eq!(revision(&cached), Some(REVISION));
    assert_eq!(body(cached).await, Bytes::from_static(CLIP));
    assert_eq!(asked.status(), StatusCode::ACCEPTED);
    assert_eq!(revision(&asked), Some(REVISION));
    let id: Value = serde_json::from_slice(&body(asked).await).expect("a JSON ticket");
    assert_eq!(id, json!({"id": ticket.to_string()}));
}

#[tokio::test]
async fn the_poll_answers_the_clip_with_the_revision_and_the_failure_or_no_content_without_it() {
    let clip = reply(Fetched::Clip(Bytes::from_static(CLIP)), REVISION).expect("a clip");
    let failed = reply(Fetched::Failed, REVISION)
        .expect_err("a failure")
        .into_response();
    let expired = reply(Fetched::Expired, REVISION)
        .expect_err("a failure")
        .into_response();
    let pending = reply(Fetched::Pending, REVISION).expect("no content yet");

    assert_eq!(clip.status(), StatusCode::OK);
    assert_eq!(content_type(&clip), Some("audio/ogg"));
    assert_eq!(revision(&clip), Some(REVISION));
    assert_eq!(body(clip).await, Bytes::from_static(CLIP));
    for answered in [&failed, &expired, &pending] {
        assert_eq!(revision(answered), None, "{}", answered.status());
    }
    assert_eq!(failed.status(), StatusCode::BAD_GATEWAY);
    let failure: Value = serde_json::from_slice(&body(failed).await).expect("a JSON error");
    assert_eq!(failure["code"], "voice_speak_failed");
    assert_eq!(expired.status(), StatusCode::BAD_GATEWAY);
    let lost: Value = serde_json::from_slice(&body(expired).await).expect("a JSON error");
    assert_eq!(lost["code"], "voice_speak_failed");
    assert_eq!(pending.status(), StatusCode::NO_CONTENT);
    assert!(body(pending).await.is_empty());
}

#[tokio::test]
async fn a_ticket_that_is_not_a_v7_uuid_is_not_found() {
    let ticket = Uuid::now_v7();

    let missing = ticket_of("not-a-ticket")
        .expect_err("not a ticket")
        .into_response();

    assert_eq!(missing.status(), StatusCode::NOT_FOUND);
    let error: Value = serde_json::from_slice(&body(missing).await).expect("a JSON error");
    assert_eq!(error["code"], "voice_speak_not_found");
    assert_eq!(ticket_of(&ticket.to_string()).ok(), Some(ticket));
    assert!(ticket_of("98765.0190f0c2").is_err());
    assert!(ticket_of(&Uuid::new_v4().to_string()).is_err());
    assert!(ticket_of("").is_err());
}

#[tokio::test]
async fn every_refusal_of_the_routes_carries_its_own_code() {
    let refusals = [
        (
            invalid("text is empty".to_owned()),
            StatusCode::BAD_REQUEST,
            "voice_speak_invalid",
        ),
        (busy(), StatusCode::SERVICE_UNAVAILABLE, "voice_speak_busy"),
        (
            unavailable(),
            StatusCode::SERVICE_UNAVAILABLE,
            "voice_speak_unavailable",
        ),
    ];

    for (error, status, code) in refusals {
        let response = error.into_response();
        let retry_after = response.headers().contains_key("retry-after");
        assert_eq!(revision(&response), None, "{code}");
        assert_eq!(response.status(), status, "{code}");
        assert_eq!(retry_after, code == "voice_speak_unavailable", "{code}");
        let refusal: Value = serde_json::from_slice(&body(response).await).expect("a JSON error");
        assert_eq!(refusal["code"], code);
    }
}
