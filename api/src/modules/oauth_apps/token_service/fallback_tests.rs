use std::sync::atomic::{AtomicUsize, Ordering};

use axum::extract::State;
use axum::routing::post;
use axum::{Json, Router};
use futures::future::join_all;
use serde_json::{Value, json};

use super::*;
use crate::modules::oauth_apps::OAuthAppTokenService;

async fn token(State(calls): State<Arc<AtomicUsize>>) -> Json<Value> {
    let call = calls.fetch_add(1, Ordering::SeqCst) + 1;
    tokio::time::sleep(Duration::from_millis(50)).await;
    Json(json!({
        "access_token": format!("fallback-{call}"),
        "refresh_token": "fallback-refresh",
        "expires_in": 3600,
        "scope": ""
    }))
}

async fn token_endpoint() -> anyhow::Result<(String, Arc<AtomicUsize>)> {
    let calls = Arc::new(AtomicUsize::new(0));
    let app = Router::new()
        .route("/oauth/token", post(token))
        .with_state(calls.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    tokio::spawn(async move { axum::serve(listener, app).await });
    Ok((format!("http://{address}"), calls))
}

fn process(
    pool: &PgPool,
    auth_base: &str,
    core_down_for: Option<Duration>,
) -> anyhow::Result<Arc<AppTokenFallback>> {
    let url = std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".to_owned());
    let redis = deadpool_redis::Config::from_url(url)
        .create_pool(Some(deadpool_redis::Runtime::Tokio1))?;
    let sc = ScClient::new(&sc_transport::ScConfig {
        proxy_url: "http://127.0.0.1:1".into(),
        proxy_fallback: false,
        api_base: None,
        home_base: None,
    })?
    .with_auth_base(auth_base);
    let fallback = AppTokenFallback::new(pool.clone(), redis, sc, CoreRpc::unreachable());
    fallback
        .clock
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .core_down_since = core_down_for.and_then(|down_for| Instant::now().checked_sub(down_for));
    Ok(fallback)
}

async fn app(pool: &PgPool) -> anyhow::Result<Uuid> {
    let id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO oauth_apps (id, name, client_id, client_secret, redirect_uri)
         VALUES ($1, 'app', $1::text, 'secret', 'https://app.test/callback')",
    )
    .bind(id)
    .execute(pool)
    .await?;
    Ok(id)
}

async fn local_tokens(pool: &PgPool, app: Uuid) -> anyhow::Result<i64> {
    Ok(sqlx::query_scalar(
        "SELECT count(*) FROM oauth_app_tokens_local WHERE oauth_app_id = $1 AND expires_at > now()",
    )
    .bind(app)
    .fetch_one(pool)
    .await?)
}

#[sqlx::test(migrations = "./migrations")]
async fn racing_processes_issue_one_token_per_app(pool: PgPool) -> anyhow::Result<()> {
    let (auth_base, calls) = token_endpoint().await?;
    let apps = [app(&pool).await?, app(&pool).await?];
    let processes = [
        process(&pool, &auth_base, Some(CORE_DOWN_FOR))?,
        process(&pool, &auth_base, Some(CORE_DOWN_FOR))?,
    ];

    join_all(processes.iter().cycle().take(40).map(|process| process.top_up())).await;
    for process in &processes {
        process.clock.lock().unwrap_or_else(|error| error.into_inner()).probed_at = None;
    }
    join_all(processes.iter().map(|process| process.top_up())).await;

    assert_eq!(calls.load(Ordering::SeqCst), apps.len());
    for app in apps {
        assert_eq!(local_tokens(&pool, app).await?, 1);
    }
    let served = OAuthAppTokenService::new(pool.clone()).snapshot().await?;
    assert_eq!(served.len(), apps.len());
    assert!(served.iter().all(|token| token.access_token.starts_with("fallback-")));
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn a_short_core_outage_issues_nothing(pool: PgPool) -> anyhow::Result<()> {
    let (auth_base, calls) = token_endpoint().await?;
    let app = app(&pool).await?;
    let process = process(&pool, &auth_base, None)?;

    let stored = process.top_up().await;

    assert!(!stored);
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert_eq!(local_tokens(&pool, app).await?, 0);
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn a_live_shared_token_needs_no_fallback(pool: PgPool) -> anyhow::Result<()> {
    let (auth_base, calls) = token_endpoint().await?;
    let app = app(&pool).await?;
    sqlx::query(
        "INSERT INTO oauth_app_tokens (oauth_app_id, generation, access_token, expires_at)
         VALUES ($1, gen_random_uuid(), 'shared', now() + interval '1 hour')",
    )
    .bind(app)
    .execute(&pool)
    .await?;
    let process = process(&pool, &auth_base, Some(CORE_DOWN_FOR))?;

    let stored = process.top_up().await;

    assert!(!stored);
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert_eq!(local_tokens(&pool, app).await?, 0);
    Ok(())
}
