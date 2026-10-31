use std::sync::Arc;

use sqlx::PgPool;
use uuid::Uuid;

use crate::error::AppError;
use crate::modules::auth::AuthService;
use crate::modules::auth::CoreRpc;
use crate::modules::auth::health::AuthHealthService;
use crate::modules::oauth_apps::OAuthAppsService;
use crate::sc::ScClient;

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
        CoreRpc::unreachable(),
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
async fn login_waits_when_the_only_write_app_is_penalized(pool: PgPool) -> anyhow::Result<()> {
    let (auth, health) = auth(&pool)?;
    let refused = oauth_app(&pool).await?;
    let writable = oauth_app(&pool).await?;
    refuse_writes(&pool, refused).await?;
    health.penalize_app_at_least(writable, 60).await?;

    let login = auth.initiate_login(None).await;

    assert!(matches!(
        login,
        Err(AppError::SoundCloudRefreshRateLimited { retry_after_sec }) if retry_after_sec > 0
    ));
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
