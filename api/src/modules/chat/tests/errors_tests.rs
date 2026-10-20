use axum::body::to_bytes;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use serde_json::{Value, json};

use super::ChatError;

async fn wire(error: ChatError) -> (StatusCode, Value) {
    let response = error.into_response();
    let status = response.status();
    let body = to_bytes(response.into_body(), 4096).await.expect("a body");
    (status, serde_json::from_slice(&body).expect("json"))
}

#[tokio::test]
async fn an_error_carries_its_code_and_the_fields_the_client_needs() {
    assert_eq!(
        wire(ChatError::Disabled).await,
        (
            StatusCode::SERVICE_UNAVAILABLE,
            json!({"error": "chat_disabled"})
        )
    );
    assert_eq!(
        wire(ChatError::Clock { server_now: 42 }).await,
        (
            StatusCode::UNAUTHORIZED,
            json!({"error": "chat_clock", "server_now": 42})
        )
    );
    assert_eq!(
        wire(ChatError::SlowMode { retry_ms: 2_500 }).await,
        (
            StatusCode::TOO_MANY_REQUESTS,
            json!({"error": "chat_slow_mode", "retry_ms": 2500})
        )
    );
    assert_eq!(
        wire(ChatError::Muted { until: 7 }).await,
        (
            StatusCode::FORBIDDEN,
            json!({"error": "chat_muted", "until": 7})
        )
    );
    assert_eq!(
        wire(ChatError::Unimplemented).await,
        (
            StatusCode::NOT_IMPLEMENTED,
            json!({"error": "chat_unimplemented"})
        )
    );
}

#[test]
fn every_code_is_prefixed_and_unique() {
    let all = [
        ChatError::NotMember,
        ChatError::EpochStale,
        ChatError::Muted { until: 0 },
        ChatError::SlowMode { retry_ms: 0 },
        ChatError::Unavailable,
        ChatError::RequestLimit,
        ChatError::SendLimited,
        ChatError::ReadLimited,
        ChatError::Busy { retry_ms: 0 },
        ChatError::Clock { server_now: 0 },
        ChatError::Replay,
        ChatError::DeviceRevoked,
        ChatError::ReloginRequired,
        ChatError::SessionBoundOther,
        ChatError::TooYoung,
        ChatError::Disabled,
        ChatError::SendPaused,
        ChatError::Invalid,
        ChatError::NotFound,
        ChatError::Unimplemented,
        ChatError::Internal,
    ];
    let mut codes: Vec<&str> = all.iter().map(ChatError::code).collect();
    assert!(codes.iter().all(|code| code.starts_with("chat_")));
    codes.sort_unstable();
    codes.dedup();
    assert_eq!(codes.len(), all.len());
}
