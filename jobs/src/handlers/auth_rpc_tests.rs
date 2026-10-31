use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
use serde_json::json;
use tokio::task::JoinSet;

use super::*;

#[derive(Clone)]
struct TokenEndpoint {
    calls: Arc<AtomicUsize>,
    answer: Answer,
}

#[derive(Clone, Copy)]
enum Answer {
    NewPair,
    InvalidGrant,
    RateLimited,
}

async fn token(State(endpoint): State<TokenEndpoint>) -> Response {
    let call = endpoint.calls.fetch_add(1, Ordering::SeqCst) + 1;
    tokio::time::sleep(Duration::from_millis(50)).await;
    match endpoint.answer {
        Answer::NewPair => Json(json!({
            "access_token": format!("new-access-{call}"),
            "refresh_token": format!("new-refresh-{call}"),
            "expires_in": 3600,
            "scope": "*"
        }))
        .into_response(),
        Answer::InvalidGrant => (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "invalid_grant" })),
        )
            .into_response(),
        Answer::RateLimited => (
            StatusCode::TOO_MANY_REQUESTS,
            [("retry-after", "120")],
            Json(json!({})),
        )
            .into_response(),
    }
}

async fn rpc(pool: &PgPool, answer: Answer) -> anyhow::Result<(Arc<AuthRpc>, Arc<AtomicUsize>)> {
    let calls = Arc::new(AtomicUsize::new(0));
    let app = Router::new()
        .route("/token", post(token))
        .with_state(TokenEndpoint {
            calls: calls.clone(),
            answer,
        });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    tokio::spawn(async move { axum::serve(listener, app).await });
    let oauth = OAuthConfig {
        token_url: format!("http://{address}/token").parse()?,
        bootstrap_app: None,
    };
    Ok((Arc::new(AuthRpc::new(pool.clone(), &oauth)?), calls))
}

async fn connection(pool: &PgPool, access_token: &str, expires_in: &str) -> anyhow::Result<Uuid> {
    let app_id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO oauth_apps (id, name, client_id, client_secret, redirect_uri)
         VALUES ($1, 'app', 'client', 'secret', 'https://app.test/callback')",
    )
    .bind(app_id)
    .execute(pool)
    .await?;
    let connection_id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO soundcloud_connections (
             id, soundcloud_user_id, oauth_app_id, access_token, refresh_token, expires_at, scope
         ) VALUES ($1, '42', $2, $3, 'old-refresh', now() + $4::interval, '*')",
    )
    .bind(connection_id)
    .bind(app_id)
    .bind(access_token)
    .bind(expires_in)
    .execute(pool)
    .await?;
    Ok(connection_id)
}

fn request(connection_id: Uuid, rejected: Option<&str>, hold: bool) -> RefreshRequest {
    RefreshRequest {
        connection_id,
        rejected_token_fingerprint: rejected.map(token_fingerprint),
        hold,
    }
}

async fn many(
    rpc: &Arc<AuthRpc>,
    count: usize,
    request: RefreshRequest,
) -> anyhow::Result<Vec<RefreshReply>> {
    let mut callers = JoinSet::new();
    for _ in 0..count {
        let rpc = rpc.clone();
        let request = request.clone();
        callers.spawn(async move { rpc.refresh(request).await });
    }
    let mut replies = Vec::new();
    while let Some(reply) = callers.join_next().await {
        replies.push(reply??);
    }
    Ok(replies)
}

fn token_of(reply: &RefreshReply) -> Option<&str> {
    reply
        .connection
        .as_ref()
        .map(|connection| connection.access_token.as_str())
}

#[sqlx::test(migrations = "../api/migrations")]
async fn parallel_refreshes_spend_the_refresh_token_once(pool: PgPool) -> anyhow::Result<()> {
    let (rpc, calls) = rpc(&pool, Answer::NewPair).await?;
    let connection_id = connection(&pool, "old-access", "-1 minute").await?;

    let replies = many(&rpc, 100, request(connection_id, None, false)).await?;

    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(replies.iter().all(|reply| matches!(
        reply.outcome,
        RefreshOutcome::Refreshed | RefreshOutcome::AlreadyFresh
    )));
    assert!(
        replies
            .iter()
            .all(|reply| token_of(reply) == Some("new-access-1"))
    );
    let stored: String =
        sqlx::query_scalar("SELECT refresh_token FROM soundcloud_connections WHERE id = $1")
            .bind(connection_id)
            .fetch_one(&pool)
            .await?;
    assert_eq!(stored, "new-refresh-1");
    Ok(())
}

#[sqlx::test(migrations = "../api/migrations")]
async fn a_rejected_token_is_replaced_once(pool: PgPool) -> anyhow::Result<()> {
    let (rpc, calls) = rpc(&pool, Answer::NewPair).await?;
    let connection_id = connection(&pool, "old-access", "1 hour").await?;

    let replies = many(&rpc, 20, request(connection_id, Some("old-access"), false)).await?;

    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(
        replies
            .iter()
            .all(|reply| token_of(reply) == Some("new-access-1"))
    );
    Ok(())
}

#[sqlx::test(migrations = "../api/migrations")]
async fn a_fresh_token_is_returned_without_a_refresh(pool: PgPool) -> anyhow::Result<()> {
    let (rpc, calls) = rpc(&pool, Answer::NewPair).await?;
    let connection_id = connection(&pool, "old-access", "1 hour").await?;

    let reply = rpc.refresh(request(connection_id, None, false)).await?;

    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert_eq!(reply.outcome, RefreshOutcome::AlreadyFresh);
    assert_eq!(token_of(&reply), Some("old-access"));
    Ok(())
}

#[sqlx::test(migrations = "../api/migrations")]
async fn hold_marks_the_token_and_names_the_retry(pool: PgPool) -> anyhow::Result<()> {
    let (rpc, calls) = rpc(&pool, Answer::NewPair).await?;
    let connection_id = connection(&pool, "old-access", "1 hour").await?;

    let reply = rpc
        .refresh(request(connection_id, Some("old-access"), true))
        .await?;

    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert_eq!(reply.outcome, RefreshOutcome::RetryLater);
    let state = reply.connection.expect("state comes back");
    assert_eq!(
        state.last_refresh_error_kind.as_deref(),
        Some("token_rejected")
    );
    let retry_in = state.retry_at_unix_ms.expect("retry is set") - Utc::now().timestamp_millis();
    assert!((280_000..=301_000).contains(&retry_in));
    Ok(())
}

#[sqlx::test(migrations = "../api/migrations")]
async fn one_rejected_refresh_token_is_not_a_logout(pool: PgPool) -> anyhow::Result<()> {
    let (rpc, calls) = rpc(&pool, Answer::InvalidGrant).await?;
    let connection_id = connection(&pool, "old-access", "-1 minute").await?;

    let replies = many(&rpc, 10, request(connection_id, None, false)).await?;

    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(
        replies
            .iter()
            .all(|reply| reply.outcome == RefreshOutcome::RetryLater)
    );
    let kind: Option<String> = sqlx::query_scalar(
        "SELECT last_refresh_error_kind FROM soundcloud_connections WHERE id = $1",
    )
    .bind(connection_id)
    .fetch_one(&pool)
    .await?;
    assert_eq!(kind.as_deref(), Some("temporarily_unavailable"));
    Ok(())
}

#[sqlx::test(migrations = "../api/migrations")]
async fn an_unknown_connection_is_retried_not_disconnected(pool: PgPool) -> anyhow::Result<()> {
    let (rpc, calls) = rpc(&pool, Answer::NewPair).await?;

    let reply = rpc.refresh(request(Uuid::now_v7(), None, false)).await?;

    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert_eq!(reply.outcome, RefreshOutcome::RetryLater);
    assert!(reply.connection.is_none());
    Ok(())
}

#[sqlx::test(migrations = "../api/migrations")]
async fn the_app_token_is_cores_live_token(pool: PgPool) -> anyhow::Result<()> {
    let (rpc, _) = rpc(&pool, Answer::NewPair).await?;
    let app_id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO oauth_apps (id, name, client_id, client_secret, redirect_uri)
         VALUES ($1, 'app', 'client', 'secret', 'https://app.test/callback')",
    )
    .bind(app_id)
    .execute(&pool)
    .await?;
    let empty = rpc
        .app_token(AppTokenRequest {
            oauth_app_id: app_id,
        })
        .await?;
    assert!(empty.token.is_none());

    sqlx::query(
        "INSERT INTO oauth_app_tokens (oauth_app_id, generation, access_token, expires_at)
         VALUES ($1, $2, 'app-access', now() + interval '1 hour')",
    )
    .bind(app_id)
    .bind(Uuid::from_u128(7))
    .execute(&pool)
    .await?;
    let reply = rpc
        .app_token(AppTokenRequest {
            oauth_app_id: app_id,
        })
        .await?;

    let token = reply.token.expect("token comes back");
    assert_eq!(token.access_token, "app-access");
    assert_eq!(token.generation, Uuid::from_u128(7));
    Ok(())
}

async fn app_of(pool: &PgPool, connection_id: Uuid) -> anyhow::Result<Uuid> {
    Ok(
        sqlx::query_scalar("SELECT oauth_app_id FROM soundcloud_connections WHERE id = $1")
            .bind(connection_id)
            .fetch_one(pool)
            .await?,
    )
}

async fn app_cooldown(pool: &PgPool, app_id: Uuid) -> anyhow::Result<Option<bool>> {
    Ok(sqlx::query_scalar(
        "SELECT retry_at > now() FROM oauth_app_request_cooldowns WHERE oauth_app_id = $1",
    )
    .bind(app_id)
    .fetch_optional(pool)
    .await?)
}

#[sqlx::test(migrations = "../api/migrations")]
async fn a_read_pause_does_not_hold_back_a_token_refresh(pool: PgPool) -> anyhow::Result<()> {
    let (rpc, calls) = rpc(&pool, Answer::NewPair).await?;
    let connection_id = connection(&pool, "old-access", "-1 minute").await?;
    let app_id = app_of(&pool, connection_id).await?;
    rpc.connections.pause_reads(app_id, 600);

    let reply = rpc.refresh(request(connection_id, None, false)).await?;

    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(reply.outcome, RefreshOutcome::Refreshed);
    assert_eq!(token_of(&reply), Some("new-access-1"));
    assert_eq!(app_cooldown(&pool, app_id).await?, None);
    Ok(())
}

#[sqlx::test(migrations = "../api/migrations")]
async fn a_rate_limited_refresh_pauses_the_app_in_the_database(pool: PgPool) -> anyhow::Result<()> {
    let (rpc, calls) = rpc(&pool, Answer::RateLimited).await?;
    let connection_id = connection(&pool, "old-access", "-1 minute").await?;
    let app_id = app_of(&pool, connection_id).await?;

    let reply = rpc.refresh(request(connection_id, None, false)).await?;

    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(reply.outcome, RefreshOutcome::RateLimited);
    assert!(reply.retry_after_sec.is_some_and(|seconds| seconds >= 120));
    assert_eq!(app_cooldown(&pool, app_id).await?, Some(true));
    Ok(())
}
