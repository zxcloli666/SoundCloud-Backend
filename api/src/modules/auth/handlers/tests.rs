use super::*;

#[test]
fn explicit_refresh_outcomes_keep_soundcloud_failures_separate_from_session_auth() {
    for (outcome, expected) in [
        (RefreshOutcome::Refreshed, StatusCode::OK),
        (RefreshOutcome::AlreadyFresh, StatusCode::OK),
        (RefreshOutcome::InProgress, StatusCode::ACCEPTED),
        (RefreshOutcome::RateLimited, StatusCode::TOO_MANY_REQUESTS),
        (RefreshOutcome::RetryLater, StatusCode::BAD_GATEWAY),
        (RefreshOutcome::TimedOut, StatusCode::GATEWAY_TIMEOUT),
        (
            RefreshOutcome::ReauthorizationRequired,
            StatusCode::CONFLICT,
        ),
        (RefreshOutcome::NotConnected, StatusCode::CONFLICT),
    ] {
        let response = refresh_response(RefreshAttempt {
            outcome,
            connection: None,
        });
        assert_eq!(response.status(), expected);
        assert_eq!(
            response.headers().get(header::CACHE_CONTROL),
            Some(&HeaderValue::from_static("no-store, private")),
        );
    }
}

#[tokio::test]
async fn refresh_reauthorization_has_a_stable_connection_body() -> anyhow::Result<()> {
    let response = refresh_response(RefreshAttempt {
        outcome: RefreshOutcome::ReauthorizationRequired,
        connection: None,
    });
    let body = axum::body::to_bytes(response.into_body(), 4096).await?;
    let value: serde_json::Value = serde_json::from_slice(&body)?;
    assert_eq!(
        value,
        serde_json::json!({
            "state": "reauthorization_required",
            "canUseSoundcloud": false,
            "canRefresh": false,
            "errorCode": "reauthorization_required",
            "errorMessage": "SoundCloud repeatedly rejected the refresh token"
        })
    );
    Ok(())
}

#[test]
fn auth_polling_responses_are_not_cacheable() {
    let response = no_store_json(serde_json::json!({ "status": "pending" }));
    assert_eq!(
        response.headers().get(header::CACHE_CONTROL),
        Some(&HeaderValue::from_static("no-store, private")),
    );
}
