use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use base64::Engine;
use serde_json::json;
use sqlx::PgPool;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use uuid::Uuid;

use crate::modules::auth::health::AuthHealthService;
use crate::modules::auth::{AuthService, TokenProvider};
use crate::modules::oauth_apps::{OAuthAppTokenService, OAuthAppsService};
use crate::sc::{ScClient, ScReadService};

fn auth(pool: &PgPool) -> anyhow::Result<(Arc<AuthService>, Arc<AuthHealthService>)> {
    let redis = deadpool_redis::Config::from_url("redis://127.0.0.1:1")
        .create_pool(Some(deadpool_redis::Runtime::Tokio1))?;
    let sc = ScClient::new(&sc_transport::ScConfig {
        proxy_url: "http://127.0.0.1:1".into(),
        proxy_fallback: false,
        api_base: None,
        home_base: None,
    })?;
    let health = AuthHealthService::with_database(redis, pool.clone());
    let auth = AuthService::new(
        pool.clone(),
        sc,
        OAuthAppsService::new(pool.clone()),
        health.clone(),
    );
    Ok((auth, health))
}

async fn oauth_app(pool: &PgPool) -> anyhow::Result<Uuid> {
    let app = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO oauth_apps (id, name, client_id, client_secret, redirect_uri)
         VALUES ($1, 'test', $2, 'secret', 'https://app.test/')",
    )
    .bind(app)
    .bind(app.to_string())
    .execute(pool)
    .await?;
    Ok(app)
}

async fn refuse_writes(pool: &PgPool, app: Uuid) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO sync_queue_action_pauses (oauth_app_id, action_type, paused_until, strikes, reason)
         VALUES ($1, 'like_track', now() - interval '1 hour', 1, 'subscription_required')",
    )
    .bind(app)
    .execute(pool)
    .await?;
    Ok(())
}

async fn login_app(pool: &PgPool, auth: &AuthService) -> anyhow::Result<Uuid> {
    let login = auth.initiate_login(None).await?;
    let app: String = sqlx::query_scalar("SELECT oauth_app_id FROM login_requests WHERE id = $1")
        .bind(login.login_request_id)
        .fetch_one(pool)
        .await?;
    Ok(app.parse()?)
}

#[sqlx::test(migrations = "./migrations")]
async fn login_prefers_an_app_that_can_write(pool: PgPool) -> anyhow::Result<()> {
    let (auth, _) = auth(&pool)?;
    let refused = oauth_app(&pool).await?;
    let writable = oauth_app(&pool).await?;
    refuse_writes(&pool, refused).await?;

    for _ in 0..3 {
        assert_eq!(login_app(&pool, &auth).await?, writable);
    }
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn login_hands_out_the_only_app_while_it_is_in_cooldown(pool: PgPool) -> anyhow::Result<()> {
    let (auth, health) = auth(&pool)?;
    let refused = oauth_app(&pool).await?;
    let writable = oauth_app(&pool).await?;
    refuse_writes(&pool, refused).await?;
    health.penalize_app_at_least(writable, 300).await?;

    assert_eq!(login_app(&pool, &auth).await?, writable);
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn login_picks_the_app_whose_cooldown_ends_first(pool: PgPool) -> anyhow::Result<()> {
    let (auth, health) = auth(&pool)?;
    let long = oauth_app(&pool).await?;
    let short = oauth_app(&pool).await?;
    health.penalize_app_at_least(long, 600).await?;
    health.penalize_app_at_least(short, 60).await?;

    for _ in 0..3 {
        assert_eq!(login_app(&pool, &auth).await?, short);
    }
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn login_prefers_an_app_without_cooldown(pool: PgPool) -> anyhow::Result<()> {
    let (auth, health) = auth(&pool)?;
    let penalized = oauth_app(&pool).await?;
    let free = oauth_app(&pool).await?;
    health.penalize_app_at_least(penalized, 60).await?;

    for _ in 0..3 {
        assert_eq!(login_app(&pool, &auth).await?, free);
    }
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn login_falls_back_to_all_apps_when_every_app_is_refused(
    pool: PgPool,
) -> anyhow::Result<()> {
    let (auth, _) = auth(&pool)?;
    let first = oauth_app(&pool).await?;
    let second = oauth_app(&pool).await?;
    refuse_writes(&pool, first).await?;
    refuse_writes(&pool, second).await?;

    let picked = login_app(&pool, &auth).await?;

    assert!(picked == first || picked == second);
    Ok(())
}

async fn token_server(status: u16, body: String) -> anyhow::Result<(String, Arc<AtomicUsize>)> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let base = format!("http://{}", listener.local_addr()?);
    let hits = Arc::new(AtomicUsize::new(0));
    let counter = hits.clone();
    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                return;
            };
            counter.fetch_add(1, Ordering::SeqCst);
            let body = body.clone();
            tokio::spawn(async move {
                let mut buffer = vec![0u8; 8192];
                let _ = socket.read(&mut buffer).await;
                let response = format!(
                    "HTTP/1.1 {status} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = socket.write_all(response.as_bytes()).await;
                let _ = socket.shutdown().await;
            });
        }
    });
    Ok((base, hits))
}

fn access_token(subject: &str) -> String {
    let encode = |value: serde_json::Value| {
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(value.to_string())
    };
    let expiry = chrono::Utc::now().timestamp() + 3600;
    format!(
        "{}.{}.sig",
        encode(json!({"alg": "none"})),
        encode(json!({"sub": subject, "exp": expiry}))
    )
}

fn callback_stack(
    pool: &PgPool,
    auth_base: &str,
) -> anyhow::Result<(Arc<AuthService>, Arc<AuthHealthService>, Arc<ScReadService>)> {
    let redis = deadpool_redis::Config::from_url("redis://127.0.0.1:1")
        .create_pool(Some(deadpool_redis::Runtime::Tokio1))?;
    let sc = ScClient::new(&sc_transport::ScConfig {
        proxy_url: String::new(),
        proxy_fallback: false,
        api_base: Some("http://127.0.0.1:1".into()),
        home_base: Some("http://127.0.0.1:1".into()),
    })?
    .with_auth_base(auth_base);
    let health = AuthHealthService::with_database(redis, pool.clone());
    let auth = AuthService::new(
        pool.clone(),
        sc.clone(),
        OAuthAppsService::new(pool.clone()),
        health.clone(),
    );
    let tokens = TokenProvider::new(auth.clone(), OAuthAppTokenService::new(pool.clone()));
    let public = ScReadService::new(sc, tokens, pool.clone());
    Ok((auth, health, public))
}

async fn login_state(pool: &PgPool, request: Uuid) -> anyhow::Result<(String, String)> {
    Ok(
        sqlx::query_as("SELECT state, status FROM login_requests WHERE id = $1")
            .bind(request)
            .fetch_one(pool)
            .await?,
    )
}

#[sqlx::test(migrations = "./migrations")]
async fn a_callback_exchanges_the_code_while_the_app_is_in_cooldown(
    pool: PgPool,
) -> anyhow::Result<()> {
    let body = json!({
        "access_token": access_token("soundcloud:users:77"),
        "refresh_token": "refresh",
        "expires_in": 3600,
        "scope": "*",
        "token_type": "Bearer"
    })
    .to_string();
    let (base, hits) = token_server(200, body).await?;
    let (auth, health, public) = callback_stack(&pool, &base)?;
    let app = oauth_app(&pool).await?;
    let login = auth.initiate_login(None).await?;
    health.penalize_app_at_least(app, 300).await?;
    let (state, _) = login_state(&pool, login.login_request_id).await?;

    auth.handle_callback(&public, "code", &state).await?;

    assert_eq!(hits.load(Ordering::SeqCst), 1);
    let (_, status) = login_state(&pool, login.login_request_id).await?;
    assert_eq!(status, "completed");
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn a_rejected_exchange_in_cooldown_penalizes_once_from_soundcloud(
    pool: PgPool,
) -> anyhow::Result<()> {
    let (base, hits) = token_server(429, "{}".to_owned()).await?;
    let (auth, health, public) = callback_stack(&pool, &base)?;
    let app = oauth_app(&pool).await?;
    let login = auth.initiate_login(None).await?;
    health.penalize_app_at_least(app, 300).await?;
    let (state, _) = login_state(&pool, login.login_request_id).await?;

    auth.handle_callback(&public, "code", &state).await?;

    let failures: i32 = sqlx::query_scalar(
        "SELECT failure_count FROM oauth_app_request_cooldowns WHERE oauth_app_id = $1",
    )
    .bind(app)
    .fetch_one(&pool)
    .await?;
    assert_eq!(hits.load(Ordering::SeqCst), 1);
    assert_eq!(failures, 2);
    Ok(())
}
