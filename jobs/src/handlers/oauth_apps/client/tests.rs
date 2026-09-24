use std::collections::HashMap;
use std::sync::Arc;

use axum::extract::{Form, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::routing::post;
use axum::{Json, Router};
use serde_json::json;
use tokio::sync::Mutex;
use uuid::Uuid;

use super::*;

#[derive(Clone, Default)]
struct Requests(Arc<Mutex<Vec<CapturedRequest>>>);

struct CapturedRequest {
    authorization: Option<String>,
    form: HashMap<String, String>,
}

#[test]
fn invalid_grant_is_detected_without_retaining_the_response_body() {
    assert_eq!(
        rejection_kind(StatusCode::BAD_REQUEST, br#"{"error":"invalid_grant"}"#),
        RejectionKind::InvalidGrant
    );
}

#[test]
fn shared_failures_are_classified_by_scope() {
    assert_eq!(
        rejection_kind(StatusCode::TOO_MANY_REQUESTS, b""),
        RejectionKind::RateLimited
    );
    assert_eq!(
        rejection_kind(StatusCode::BAD_GATEWAY, b""),
        RejectionKind::Upstream
    );
    assert_eq!(
        rejection_kind(StatusCode::BAD_REQUEST, br#"{"error":"invalid_client"}"#),
        RejectionKind::AppCredentials
    );
}

#[test]
fn retry_after_seconds_produces_a_future_timestamp() {
    assert!(parse_retry_after("5").is_some_and(|retry_at| retry_at > Utc::now()));
}

#[tokio::test]
async fn grant_requests_follow_soundcloud_oauth_contract() -> anyhow::Result<()> {
    let requests = Requests::default();
    let app = Router::new()
        .route("/token", post(token_endpoint))
        .with_state(requests.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let server = tokio::spawn(async move { axum::serve(listener, app).await });
    let config = OAuthConfig {
        token_url: format!("http://{address}/token").parse()?,
        bootstrap_app: None,
    };
    let client = OAuthTokenClient::new(&config)?;
    let claimed = ClaimedApp {
        id: Uuid::now_v7(),
        client_id: "client".to_owned(),
        client_secret: "secret".to_owned(),
        refresh_token: Some("old-refresh".to_owned()),
        refresh_attempts: Some(0),
        lease_id: Uuid::now_v7(),
    };

    assert!(matches!(
        client.refresh(&claimed, "old-refresh").await,
        TokenRequestOutcome::Incomplete
    ));
    let token = match client.client_credentials(&claimed).await {
        TokenRequestOutcome::ClientCredentialsSuccess(token) => token,
        _ => return Err(anyhow::anyhow!("client credentials exchange failed")),
    };
    assert_eq!(token.refresh_token.as_deref(), Some("new-refresh"));

    let captured = requests.0.lock().await;
    assert_eq!(captured.len(), 2);
    assert_eq!(
        captured[0].form.get("grant_type").map(String::as_str),
        Some("refresh_token")
    );
    assert_eq!(
        captured[0].form.get("client_secret").map(String::as_str),
        Some("secret")
    );
    assert_eq!(
        captured[1].form.get("grant_type").map(String::as_str),
        Some("client_credentials")
    );
    assert!(!captured[1].form.contains_key("client_secret"));
    assert_eq!(
        captured[1].authorization.as_deref(),
        Some("Basic Y2xpZW50OnNlY3JldA==")
    );
    drop(captured);
    server.abort();
    Ok(())
}

async fn token_endpoint(
    State(requests): State<Requests>,
    headers: HeaderMap,
    Form(form): Form<HashMap<String, String>>,
) -> (StatusCode, Json<serde_json::Value>) {
    requests.0.lock().await.push(CapturedRequest {
        authorization: headers
            .get(header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned),
        form: form.clone(),
    });
    match form.get("grant_type").map(String::as_str) {
        Some("refresh_token") => (
            StatusCode::OK,
            Json(json!({"access_token": "lost-access", "expires_in": 3600})),
        ),
        Some("client_credentials") => (
            StatusCode::OK,
            Json(json!({
                "access_token": "new-access",
                "refresh_token": "new-refresh",
                "expires_in": 3600,
                "scope": ""
            })),
        ),
        _ => (StatusCode::BAD_REQUEST, Json(json!({"error": "bad_grant"}))),
    }
}
