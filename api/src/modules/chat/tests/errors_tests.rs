use axum::body::to_bytes;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use serde_json::{Value, json};

use super::{BusySource, ChatError};

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
        wire(ChatError::BackdropCooldown { until: 9 }).await,
        (
            StatusCode::FORBIDDEN,
            json!({"error": "chat_backdrop_cooldown", "until": 9})
        )
    );
    assert_eq!(
        wire(ChatError::MediaGone).await,
        (StatusCode::GONE, json!({"error": "chat_media_gone"}))
    );
    assert_eq!(
        wire(ChatError::DeviceCap).await,
        (StatusCode::FORBIDDEN, json!({"error": "chat_device_cap"}))
    );
}

#[tokio::test]
async fn the_media_errors_carry_the_fields_the_contract_names() {
    let id = uuid::Uuid::from_u128(7);
    assert_eq!(
        wire(ChatError::QuotaExceeded { used: 5, quota: 9 }).await,
        (
            StatusCode::FORBIDDEN,
            json!({"error": "chat_quota_exceeded", "used": 5, "quota": 9})
        )
    );
    assert_eq!(
        wire(ChatError::MediaMissing { ids: vec![id] }).await,
        (
            StatusCode::CONFLICT,
            json!({"error": "chat_media_missing", "ids": [id.to_string()]})
        )
    );
    assert_eq!(
        wire(ChatError::MediaUploadsPaused).await,
        (
            StatusCode::SERVICE_UNAVAILABLE,
            json!({"error": "chat_media_uploads_paused"})
        )
    );
    assert_eq!(
        wire(ChatError::MediaStarRequired).await,
        (
            StatusCode::FORBIDDEN,
            json!({"error": "chat_media_star_required"})
        )
    );
    assert_eq!(
        wire(ChatError::RoomStarRequired).await,
        (
            StatusCode::FORBIDDEN,
            json!({"error": "chat_room_star_required"})
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
        ChatError::DeviceCap,
        ChatError::SendLimited,
        ChatError::ReadLimited,
        ChatError::Busy {
            retry_ms: 0,
            source: BusySource::Channel,
        },
        ChatError::Clock { server_now: 0 },
        ChatError::Replay,
        ChatError::DeviceRevoked,
        ChatError::ReloginRequired,
        ChatError::SessionNotReplicated,
        ChatError::SessionBoundOther,
        ChatError::TooYoung,
        ChatError::Disabled,
        ChatError::SendPaused,
        ChatError::QuotaExceeded { used: 0, quota: 0 },
        ChatError::MediaStarRequired,
        ChatError::RoomStarRequired,
        ChatError::MediaUploadsPaused,
        ChatError::MediaMissing { ids: Vec::new() },
        ChatError::Invalid,
        ChatError::NotFound,
        ChatError::BackdropCooldown { until: 0 },
        ChatError::MediaGone,
        ChatError::Internal,
    ];
    let mut codes: Vec<&str> = all.iter().map(ChatError::code).collect();
    assert!(codes.iter().all(|code| code.starts_with("chat_")));
    codes.sort_unstable();
    codes.dedup();
    assert_eq!(codes.len(), all.len());
}

#[test]
fn a_busy_answer_spreads_the_retries_of_a_crowd() {
    let waits: std::collections::HashSet<u64> = (0..200)
        .map(|_| match ChatError::busy(BusySource::RedisCommand) {
            ChatError::Busy { retry_ms, .. } => retry_ms,
            other => panic!("{other:?}"),
        })
        .collect();
    assert!(waits.iter().all(|wait| (1_000..3_000).contains(wait)));
    assert!(waits.len() > 50, "{} distinct waits", waits.len());
}

#[test]
fn every_busy_source_keeps_the_same_wire_body_under_its_own_label() {
    let sources = [
        BusySource::ColdSession,
        BusySource::ColdDevice,
        BusySource::RedisPool,
        BusySource::RedisCommand,
        BusySource::PgPool,
        BusySource::S3,
        BusySource::AuthGate,
        BusySource::Rooms,
        BusySource::ConversationCreate,
        BusySource::BackdropLock,
        BusySource::StarCheck,
        BusySource::MediaReserve,
        BusySource::MediaCommit,
        BusySource::LoadtestLease,
        BusySource::AdminRooms,
        BusySource::Channel,
    ];
    for source in sources {
        let body = ChatError::busy(source).body();
        let mut keys: Vec<&str> = body
            .as_object()
            .map(|fields| fields.keys().map(String::as_str).collect())
            .unwrap_or_default();
        keys.sort_unstable();
        assert_eq!(keys, ["error", "retry_ms"], "{source:?}");
        assert_eq!(body["error"], "chat_busy");
    }
    let mut labels: Vec<&str> = sources.iter().map(|source| source.label()).collect();
    labels.sort_unstable();
    labels.dedup();
    assert_eq!(labels.len(), sources.len());
}

#[test]
fn a_pool_timeout_is_busy_and_a_failed_query_is_internal() {
    assert!(matches!(
        ChatError::from(sqlx::Error::PoolTimedOut),
        ChatError::Busy {
            source: BusySource::PgPool,
            ..
        }
    ));
    assert_eq!(
        ChatError::from(sqlx::Error::RowNotFound),
        ChatError::Internal
    );
}
