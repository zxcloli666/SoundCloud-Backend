use axum::body::to_bytes;
use axum::http::{StatusCode, header};
use axum::response::IntoResponse;
use serde_json::{Value, json};

use crate::cache::ListPageResult;
use crate::error::AppError;
use crate::modules::cold_refresh::collection::{CollectionPage, CollectionSync};
use crate::modules::playlists::PlaylistMembershipStatus;

fn wire<T: serde::Serialize>(value: &T) -> Value {
    serde_json::to_value(value).expect("contract value must serialize")
}

async fn error_wire(error: AppError) -> (StatusCode, Option<String>, Value) {
    let response = error.into_response();
    let status = response.status();
    let retry_after = response
        .headers()
        .get(header::RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    let body = to_bytes(response.into_body(), 64 * 1024)
        .await
        .expect("error body must be readable");
    (status, retry_after, serde_json::from_slice(&body).unwrap())
}

fn sync() -> CollectionSync {
    CollectionSync {
        status: "refreshing",
        last_completed_at: Some(
            chrono::DateTime::parse_from_rfc3339("2026-09-12T10:00:00Z")
                .unwrap()
                .with_timezone(&chrono::Utc),
        ),
        retry_after_seconds: 5,
    }
}

fn sc_card() -> Value {
    json!({
        "urn": "soundcloud:users:42",
        "username": "Artist",
        "avatar_url": "https://example.invalid/a.jpg",
        "followers_count": 7
    })
}

#[test]
fn a_collection_page_passes_soundcloud_cards_through_untouched() {
    let page = CollectionPage::new(
        ListPageResult {
            collection: vec![sc_card()],
            page: 2,
            page_size: 30,
            has_more: true,
        },
        sync(),
    );

    assert_eq!(
        wire(&page),
        json!({
            "collection": [{
                "urn": "soundcloud:users:42",
                "username": "Artist",
                "avatar_url": "https://example.invalid/a.jpg",
                "followers_count": 7
            }],
            "page": 2,
            "pageSize": 30,
            "page_size": 30,
            "hasMore": true,
            "has_more": true,
            "sync": {
                "status": "refreshing",
                "lastCompletedAt": "2026-09-12T10:00:00Z",
                "retryAfterSeconds": 5
            }
        })
    );
}

#[test]
fn an_empty_collection_page_keeps_every_field_and_a_null_completion() {
    let page = CollectionPage::empty(
        CollectionSync {
            status: "ready",
            last_completed_at: None,
            retry_after_seconds: 0,
        },
        0,
        30,
    );

    assert_eq!(
        wire(&page),
        json!({
            "collection": [],
            "page": 0,
            "pageSize": 30,
            "page_size": 30,
            "hasMore": false,
            "has_more": false,
            "sync": {
                "status": "ready",
                "lastCompletedAt": null,
                "retryAfterSeconds": 0
            }
        })
    );
}

#[test]
fn a_collection_page_reports_paging_under_both_spellings_with_one_value() {
    for (page_size, has_more) in [(30, true), (200, false), (1, true)] {
        let page = wire(&CollectionPage::new(
            ListPageResult {
                collection: vec![sc_card()],
                page: 1,
                page_size,
                has_more,
            },
            sync(),
        ));
        let object = page.as_object().expect("collection page is an object");

        for key in ["pageSize", "page_size", "hasMore", "has_more"] {
            assert!(object.contains_key(key), "missing {key}");
        }
        assert_eq!(page["pageSize"], json!(page_size));
        assert_eq!(page["page_size"], page["pageSize"]);
        assert_eq!(page["hasMore"], json!(has_more));
        assert_eq!(page["has_more"], page["hasMore"]);
    }
}

#[test]
fn a_plain_list_page_stays_snake_case() {
    let page = ListPageResult {
        collection: vec![json!({"urn": "soundcloud:tracks:1", "user_favorite": true})],
        page: 0,
        page_size: 30,
        has_more: false,
    };

    assert_eq!(
        wire(&page),
        json!({
            "collection": [{"urn": "soundcloud:tracks:1", "user_favorite": true}],
            "page": 0,
            "page_size": 30,
            "has_more": false
        })
    );
}

#[test]
fn playlist_membership_status_stays_camel_case_with_every_counter() {
    let status = PlaylistMembershipStatus {
        baseline_generation: 3,
        projection_revision: 9,
        projection_track_count: 28,
        last_operation_sequence: 5,
        committed_operation_sequence: 4,
        pending_operations: 1,
        conflicted_operations: 0,
        status: "shadow_ready".to_owned(),
        conflict_code: None,
        observed_at: Some(
            chrono::DateTime::parse_from_rfc3339("2026-09-12T10:00:00Z")
                .unwrap()
                .with_timezone(&chrono::Utc),
        ),
    };

    assert_eq!(
        wire(&status),
        json!({
            "baselineGeneration": 3,
            "projectionRevision": 9,
            "projectionTrackCount": 28,
            "lastOperationSequence": 5,
            "committedOperationSequence": 4,
            "pendingOperations": 1,
            "conflictedOperations": 0,
            "status": "shadow_ready",
            "conflictCode": null,
            "observedAt": "2026-09-12T10:00:00Z"
        })
    );
}

#[test]
fn a_session_response_omits_what_it_does_not_know() {
    use crate::modules::auth::dto::SessionResponse;

    assert_eq!(
        wire(&SessionResponse {
            authenticated: false,
            session_id: None,
            username: None,
            soundcloud_user_id: None,
            expires_at: None,
        }),
        json!({"authenticated": false})
    );

    assert_eq!(
        wire(&SessionResponse {
            authenticated: true,
            session_id: Some(uuid::Uuid::nil()),
            username: Some("Artist".to_owned()),
            soundcloud_user_id: Some("42".to_owned()),
            expires_at: Some(
                chrono::DateTime::parse_from_rfc3339("2026-09-12T10:00:00Z")
                    .unwrap()
                    .with_timezone(&chrono::Utc)
            ),
        }),
        json!({
            "authenticated": true,
            "sessionId": "00000000-0000-0000-0000-000000000000",
            "username": "Artist",
            "soundcloudUserId": "42",
            "expiresAt": "2026-09-12T10:00:00Z"
        })
    );
}

#[test]
fn every_soundcloud_connection_state_keeps_its_wire_name() {
    use crate::modules::auth::dto::SoundCloudConnectionState as State;

    let names: Vec<Value> = [
        State::Ready,
        State::RefreshDue,
        State::Refreshing,
        State::RetryLater,
        State::ReauthorizationRequired,
        State::NotConnected,
    ]
    .iter()
    .map(wire)
    .collect();

    assert_eq!(
        names,
        [
            json!("ready"),
            json!("refresh_due"),
            json!("refreshing"),
            json!("retry_later"),
            json!("reauthorization_required"),
            json!("not_connected")
        ]
    );
}

#[test]
fn a_disconnected_account_reports_only_the_three_decision_fields() {
    use crate::modules::auth::dto::{SoundCloudConnectionResponse, SoundCloudConnectionState};

    assert_eq!(
        wire(&SoundCloudConnectionResponse {
            state: SoundCloudConnectionState::NotConnected,
            can_use_soundcloud: false,
            can_refresh: false,
            soundcloud_user_id: None,
            soundcloud_urn: None,
            access_token_expires_at: None,
            expires_in_sec: None,
            last_attempt_at: None,
            last_success_at: None,
            retry_after_sec: None,
            error_code: None,
            error_message: None,
        }),
        json!({
            "state": "not_connected",
            "canUseSoundcloud": false,
            "canRefresh": false
        })
    );
}

#[tokio::test]
async fn a_coded_error_carries_its_code_and_retry_after() {
    let (status, retry_after, body) = error_wire(
        AppError::coded(
            StatusCode::SERVICE_UNAVAILABLE,
            "list_page_pending",
            "List page is not ready; retry the same page",
        )
        .with_retry_after(5),
    )
    .await;

    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(retry_after.as_deref(), Some("5"));
    assert_eq!(
        body,
        json!({
            "statusCode": 503,
            "code": "list_page_pending",
            "message": "List page is not ready; retry the same page",
            "error": "Service Unavailable"
        })
    );
}

#[tokio::test]
async fn an_uncoded_error_never_grows_a_code_field() {
    let (status, retry_after, body) = error_wire(AppError::not_found("Track not found")).await;

    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(retry_after, None);
    assert_eq!(
        body,
        json!({
            "statusCode": 404,
            "message": "Track not found",
            "error": "Not Found"
        })
    );
}

#[tokio::test]
async fn a_bad_request_keeps_its_public_message() {
    let (status, _, body) = error_wire(AppError::bad_request("comment body is missing")).await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(
        body,
        json!({
            "statusCode": 400,
            "message": "comment body is missing",
            "error": "Bad Request"
        })
    );
}
