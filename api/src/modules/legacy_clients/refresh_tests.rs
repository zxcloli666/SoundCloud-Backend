use axum::body::to_bytes;
use axum::http::header;
use chrono::{DateTime, Duration, Utc};
use serde_json::{Value, json};

use super::*;
use crate::modules::auth::model::SoundCloudConnection;

const SESSION: Uuid = Uuid::from_u128(0x0199_7a3c_4d2e_7000_8000_0000_0000_0017);
const OAUTH_APP: Uuid = Uuid::from_u128(0xaaaa_aaaa_aaaa_4aaa_8aaa_aaaa_aaaa_aaaa);
const EVERY_OUTCOME: [RefreshOutcome; 8] = [
    RefreshOutcome::Refreshed,
    RefreshOutcome::AlreadyFresh,
    RefreshOutcome::InProgress,
    RefreshOutcome::RateLimited,
    RefreshOutcome::RetryLater,
    RefreshOutcome::TimedOut,
    RefreshOutcome::ReauthorizationRequired,
    RefreshOutcome::NotConnected,
];

#[tokio::test]
async fn a_renewed_session_answers_200_with_the_body_old_clients_parse() -> anyhow::Result<()> {
    for outcome in [RefreshOutcome::Refreshed, RefreshOutcome::AlreadyFresh] {
        let (status, _, body) = wire(answer(SESSION, Ok(attempt(outcome, fresh())))).await?;
        assert_eq!(status, StatusCode::OK, "{outcome:?}");
        assert_eq!(
            body,
            json!({
                "sessionId": "01997a3c-4d2e-7000-8000-000000000017",
                "expiresAt": "2026-09-25T12:00:00"
            })
        );
    }
    Ok(())
}

#[tokio::test]
async fn a_refresh_already_in_flight_keeps_the_old_client_signed_in() -> anyhow::Result<()> {
    let (status, _, body) = wire(answer(
        SESSION,
        Ok(attempt(RefreshOutcome::InProgress, fresh())),
    ))
    .await?;
    assert_eq!(status, StatusCode::ACCEPTED);
    assert_eq!(body["sessionId"], "01997a3c-4d2e-7000-8000-000000000017");
    Ok(())
}

#[tokio::test]
async fn a_missing_local_session_asks_the_old_client_to_sign_in() -> anyhow::Result<()> {
    let (status, retry_after, body) = wire(answer(
        SESSION,
        Err(AppError::unauthorized("Session not found")),
    ))
    .await?;
    assert_eq!(
        (status, retry_after, body["message"].as_str()),
        (StatusCode::UNAUTHORIZED, None, Some("Session not found"))
    );
    Ok(())
}

#[tokio::test]
async fn a_rejected_soundcloud_grant_is_the_401_old_clients_sign_in_on() -> anyhow::Result<()> {
    let cases = [
        (
            Ok(attempt(
                RefreshOutcome::ReauthorizationRequired,
                failing("reauthorization_required"),
            )),
            "soundcloud_reauthorization_required",
        ),
        (
            Ok(RefreshAttempt {
                outcome: RefreshOutcome::NotConnected,
                connection: None,
            }),
            "soundcloud_not_connected",
        ),
        (
            Err(AppError::soundcloud_reauthorization_required()),
            "soundcloud_reauthorization_required",
        ),
    ];
    for (refreshed, code) in cases {
        let (status, retry_after, body) = wire(answer(SESSION, refreshed)).await?;
        assert_eq!(
            (status, retry_after, body["code"].as_str()),
            (StatusCode::UNAUTHORIZED, None, Some(code))
        );
    }
    Ok(())
}

#[tokio::test]
async fn the_sign_in_verdict_is_a_plain_coded_401_without_a_pause() -> anyhow::Result<()> {
    let (status, retry_after, body) = wire(answer(
        SESSION,
        Ok(attempt(
            RefreshOutcome::ReauthorizationRequired,
            failing("reauthorization_required"),
        )),
    ))
    .await?;
    assert_eq!((status, retry_after), (StatusCode::UNAUTHORIZED, None));
    assert_eq!(
        body,
        json!({
            "statusCode": 401,
            "code": "soundcloud_reauthorization_required",
            "message": "SoundCloud rejected the refresh token, sign in again",
            "error": "Unauthorized"
        })
    );
    Ok(())
}

#[tokio::test]
async fn a_transient_failure_stays_a_5xx_the_old_client_quietly_retries() -> anyhow::Result<()> {
    let cases = [
        (
            Ok(attempt(
                RefreshOutcome::RetryLater,
                failing("temporarily_unavailable"),
            )),
            StatusCode::BAD_GATEWAY,
            Some("121"),
        ),
        (
            Ok(attempt(RefreshOutcome::TimedOut, failing("timed_out"))),
            StatusCode::GATEWAY_TIMEOUT,
            Some("121"),
        ),
        (
            Err(AppError::soundcloud_temporarily_unavailable_for(Some(1800))),
            StatusCode::BAD_GATEWAY,
            Some("1800"),
        ),
        (
            Err(AppError::Db(sqlx::Error::PoolTimedOut)),
            StatusCode::SERVICE_UNAVAILABLE,
            None,
        ),
    ];
    for (refreshed, expected_status, expected_retry_after) in cases {
        let (status, retry_after, _) = wire(answer(SESSION, refreshed)).await?;
        assert_eq!(
            (status, retry_after.as_deref()),
            (expected_status, expected_retry_after)
        );
    }
    Ok(())
}

#[tokio::test]
async fn a_rate_limited_refresh_stays_429_with_its_pause() -> anyhow::Result<()> {
    let (status, retry_after, body) = wire(answer(
        SESSION,
        Ok(attempt(
            RefreshOutcome::RateLimited,
            failing("rate_limited"),
        )),
    ))
    .await?;
    assert_eq!(
        (status, retry_after.as_deref(), body["errorCode"].as_str()),
        (
            StatusCode::TOO_MANY_REQUESTS,
            Some("121"),
            Some("rate_limited")
        )
    );
    Ok(())
}

#[tokio::test]
async fn only_the_sign_in_verdict_and_the_success_body_differ_from_the_new_endpoint()
-> anyhow::Result<()> {
    for outcome in EVERY_OUTCOME {
        let connection = failing("temporarily_unavailable");
        let old = wire(answer(SESSION, Ok(attempt(outcome, connection.clone())))).await?;
        let new = wire(Ok(refresh_response(attempt(outcome, connection)))).await?;
        match outcome {
            RefreshOutcome::ReauthorizationRequired | RefreshOutcome::NotConnected => {
                assert_eq!(
                    (old.0, new.0),
                    (StatusCode::UNAUTHORIZED, StatusCode::CONFLICT)
                );
            }
            RefreshOutcome::Refreshed
            | RefreshOutcome::AlreadyFresh
            | RefreshOutcome::InProgress => {
                assert_eq!(old.0, new.0, "{outcome:?}");
            }
            RefreshOutcome::RateLimited | RefreshOutcome::RetryLater | RefreshOutcome::TimedOut => {
                assert_eq!(old, new, "{outcome:?}");
            }
        }
    }
    Ok(())
}

async fn wire(answer: AppResult<Response>) -> anyhow::Result<(StatusCode, Option<String>, Value)> {
    let response = answer.into_response();
    let status = response.status();
    let retry_after = response
        .headers()
        .get(header::RETRY_AFTER)
        .map(|value| value.to_str().map(str::to_owned))
        .transpose()?;
    let body = to_bytes(response.into_body(), 64 * 1024).await?;
    Ok((status, retry_after, serde_json::from_slice(&body)?))
}

fn attempt(outcome: RefreshOutcome, connection: SoundCloudConnection) -> RefreshAttempt {
    RefreshAttempt {
        outcome,
        connection: Some(connection),
    }
}

fn fresh() -> SoundCloudConnection {
    let expires_at = "2026-09-25T12:00:00Z"
        .parse()
        .expect("a fixed RFC 3339 instant parses");
    connection(expires_at, None, None)
}

fn failing(kind: &str) -> SoundCloudConnection {
    connection(half_past(-3600), Some(kind), Some(half_past(120)))
}

fn half_past(seconds: i64) -> DateTime<Utc> {
    Utc::now() + Duration::seconds(seconds) + Duration::milliseconds(500)
}

fn connection(
    expires_at: DateTime<Utc>,
    error_kind: Option<&str>,
    retry_at: Option<DateTime<Utc>>,
) -> SoundCloudConnection {
    SoundCloudConnection {
        id: Uuid::nil(),
        soundcloud_user_id: "42".to_owned(),
        oauth_app_id: Some(OAUTH_APP),
        access_token: "access".to_owned(),
        refresh_token: "refresh".to_owned(),
        expires_at,
        scope: String::new(),
        refresh_generation: 1,
        refresh_failure_count: 1,
        refresh_lease_id: None,
        refresh_lease_expires_at: None,
        last_refresh_attempt_at: None,
        last_refresh_success_at: None,
        last_refresh_error_kind: error_kind.map(str::to_owned),
        last_refresh_error: error_kind.map(|kind| format!("{kind} on the last attempt")),
        retry_at,
    }
}
