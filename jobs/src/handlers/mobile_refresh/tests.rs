use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use axum::Router;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use serde_json::json;
use tokio::task::JoinSet;

use super::*;

const ACCOUNT: &str = "4242";

#[derive(Clone)]
struct FakeProxy {
    calls: Arc<AtomicUsize>,
    answer: Answer,
}

#[derive(Clone, Copy)]
enum Answer {
    Rotated,
    InvalidGrant,
    InvalidClient,
    Captcha,
}

async fn token(State(proxy): State<FakeProxy>, headers: HeaderMap) -> Response {
    let target = headers
        .get("x-target")
        .and_then(|value| STANDARD.decode(value.as_bytes()).ok())
        .and_then(|value| String::from_utf8(value).ok());
    if target.as_deref() != Some(sc_mobile::TOKEN_URL) {
        return StatusCode::NOT_FOUND.into_response();
    }
    let call = proxy.calls.fetch_add(1, Ordering::SeqCst) + 1;
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    match proxy.answer {
        Answer::Rotated => axum::Json(json!({
            "access_token": format!("fake-access-{call}"),
            "refresh_token": format!("fake-refresh-{call}"),
            "expires_in": 604799,
            "scope": "* fast-connect playcount upload signup purchase"
        }))
        .into_response(),
        Answer::InvalidGrant => (
            StatusCode::BAD_REQUEST,
            axum::Json(json!({ "error": "invalid_grant" })),
        )
            .into_response(),
        Answer::InvalidClient => (
            StatusCode::UNAUTHORIZED,
            axum::Json(json!({ "error": "invalid_client" })),
        )
            .into_response(),
        Answer::Captcha => (
            StatusCode::FORBIDDEN,
            axum::Json(json!({
                "url": "https://geo.captcha-delivery.com/interstitial/?initialCid=fake"
            })),
        )
            .into_response(),
    }
}

async fn refresher(
    pool: &PgPool,
    answer: Answer,
) -> anyhow::Result<(Arc<MobileRefresh>, Arc<AtomicUsize>)> {
    let calls = Arc::new(AtomicUsize::new(0));
    let app = Router::new().fallback(token).with_state(FakeProxy {
        calls: calls.clone(),
        answer,
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    tokio::spawn(async move { axum::serve(listener, app).await });
    let proxy = Url::parse(&format!("http://{address}/"))?;
    Ok((
        Arc::new(MobileRefresh::new(pool.clone(), Some(proxy))),
        calls,
    ))
}

async fn linked(pool: &PgPool, expires_in: &str) -> anyhow::Result<Uuid> {
    let app_id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO oauth_apps (
             id, name, client_id, client_secret, redirect_uri,
             client_kind, app_id, app_version, user_agent
         ) VALUES (
             $1, 'mobile', 'fake-client', 'fake-secret', 'https://app.test/callback',
             'mobile', 'fake.app', '1.0.0', 'FakeAgent/1.0'
         )",
    )
    .bind(app_id)
    .execute(pool)
    .await?;
    let connection_id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO soundcloud_connections (
             id, soundcloud_user_id, oauth_app_id, access_token, refresh_token, expires_at, scope
         ) VALUES ($1, $2, $3, 'web-access', 'web-refresh', now() + interval '1 hour', '*')",
    )
    .bind(connection_id)
    .bind(ACCOUNT)
    .bind(app_id)
    .execute(pool)
    .await?;
    sqlx::query(
        "INSERT INTO mobile_credentials (
             soundcloud_user_id, oauth_app_id, device_id, access_token, refresh_token,
             expires_at, scope, linked_at
         ) VALUES ($1, $2, $3, 'old-access', 'old-refresh', now() + $4::interval, '*', now())",
    )
    .bind(ACCOUNT)
    .bind(app_id)
    .bind(Uuid::new_v4())
    .bind(expires_in)
    .execute(pool)
    .await?;
    Ok(connection_id)
}

async fn stored(pool: &PgPool) -> anyhow::Result<(String, String, String, Option<String>)> {
    let row: (String, String, String, Option<String>) = sqlx::query_as(
        "SELECT access_token, refresh_token, status, last_refresh_error_kind
         FROM mobile_credentials WHERE soundcloud_user_id = $1",
    )
    .bind(ACCOUNT)
    .fetch_one(pool)
    .await?;
    Ok(row)
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn parallel_refreshes_spend_the_refresh_token_once_and_keep_the_rotated_pair(
    pool: PgPool,
) -> anyhow::Result<()> {
    let (refresher, calls) = refresher(&pool, Answer::Rotated).await?;
    let connection_id = linked(&pool, "-1 minute").await?;

    let mut callers = JoinSet::new();
    for _ in 0..20 {
        let refresher = refresher.clone();
        callers.spawn(async move { refresher.refresh(connection_id).await });
    }
    let mut outcomes = Vec::new();
    while let Some(answer) = callers.join_next().await {
        outcomes.push(answer??.outcome);
    }

    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(outcomes.contains(&RefreshOutcome::Refreshed));
    assert!(outcomes.iter().all(|outcome| matches!(
        outcome,
        RefreshOutcome::Refreshed | RefreshOutcome::AlreadyFresh | RefreshOutcome::InProgress
    )));
    let (access, refresh, status, error) = stored(&pool).await?;
    assert_eq!(
        (access.as_str(), refresh.as_str(), status.as_str()),
        ("fake-access-1", "fake-refresh-1", "active")
    );
    assert!(error.is_none());
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn a_fresh_credential_is_not_refreshed(pool: PgPool) -> anyhow::Result<()> {
    let (refresher, calls) = refresher(&pool, Answer::Rotated).await?;
    let connection_id = linked(&pool, "6 days").await?;

    let answer = refresher.refresh(connection_id).await?;

    assert_eq!(answer.outcome, RefreshOutcome::AlreadyFresh);
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert!(answer.state.is_some_and(|state| state.status == "active"));
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn a_rejected_refresh_token_asks_for_a_relink_and_stops_refreshing(
    pool: PgPool,
) -> anyhow::Result<()> {
    let (refresher, calls) = refresher(&pool, Answer::InvalidGrant).await?;
    let connection_id = linked(&pool, "-1 minute").await?;

    let first = refresher.refresh(connection_id).await?;
    let second = refresher.refresh(connection_id).await?;

    assert_eq!(first.outcome, RefreshOutcome::ReauthorizationRequired);
    assert_eq!(second.outcome, RefreshOutcome::ReauthorizationRequired);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let (access, _, status, error) = stored(&pool).await?;
    assert_eq!(access, "old-access");
    assert_eq!(status, "relink_required");
    assert_eq!(error.as_deref(), Some("relink_required"));
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn a_captcha_is_a_refusal_retried_later_without_a_relink(pool: PgPool) -> anyhow::Result<()> {
    let (refresher, calls) = refresher(&pool, Answer::Captcha).await?;
    let connection_id = linked(&pool, "-1 minute").await?;

    let first = refresher.refresh(connection_id).await?;
    let second = refresher.refresh(connection_id).await?;

    assert_eq!(first.outcome, RefreshOutcome::RetryLater);
    assert!(first.retry_after_sec.is_some_and(|seconds| seconds >= 30));
    assert_eq!(second.outcome, RefreshOutcome::RetryLater);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let (_, _, status, error) = stored(&pool).await?;
    assert_eq!(status, "active");
    assert_eq!(error.as_deref(), Some("temporarily_unavailable"));
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn a_rejected_client_cools_the_app_down_and_keeps_the_link(
    pool: PgPool,
) -> anyhow::Result<()> {
    let (refresher, calls) = refresher(&pool, Answer::InvalidClient).await?;
    let connection_id = linked(&pool, "-1 minute").await?;

    let answer = refresher.refresh(connection_id).await?;

    assert_eq!(answer.outcome, RefreshOutcome::RetryLater);
    assert!(
        answer
            .retry_after_sec
            .is_some_and(|seconds| seconds >= 30 * 60 - 5)
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let (_, _, status, error) = stored(&pool).await?;
    assert_eq!(status, "active");
    assert_eq!(error.as_deref(), Some("app_credentials"));
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn a_session_without_a_mobile_link_is_not_connected(pool: PgPool) -> anyhow::Result<()> {
    let (refresher, calls) = refresher(&pool, Answer::Rotated).await?;

    let answer = refresher.refresh(Uuid::now_v7()).await?;

    assert_eq!(answer.outcome, RefreshOutcome::NotConnected);
    assert!(answer.state.is_none());
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    Ok(())
}
