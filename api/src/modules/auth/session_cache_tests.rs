use std::sync::Arc;
use std::time::Duration;

use sqlx::PgPool;
use uuid::Uuid;

use crate::modules::auth::{AuthHealthService, AuthService};
use crate::modules::oauth_apps::OAuthAppsService;
use crate::sc::ScClient;

const CONNECTION: Uuid = Uuid::from_u128(0xc1);
const SESSION: Uuid = Uuid::from_u128(0xa1);

fn auth(pool: &PgPool) -> anyhow::Result<Arc<AuthService>> {
    let redis = deadpool_redis::Config::from_url("redis://127.0.0.1:1")
        .create_pool(Some(deadpool_redis::Runtime::Tokio1))?;
    let sc = ScClient::new(&sc_transport::ScConfig {
        proxy_url: "http://127.0.0.1:1".into(),
        proxy_fallback: false,
        api_base: None,
        home_base: None,
    })?;
    Ok(AuthService::new(
        pool.clone(),
        sc,
        OAuthAppsService::new(pool.clone()),
        AuthHealthService::with_database(redis, pool.clone()),
    ))
}

async fn seed(pool: &PgPool) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO soundcloud_connections
            (id, soundcloud_user_id, access_token, refresh_token, expires_at, scope)
         VALUES ($1, '42', 'access', 'refresh', now() + interval '1 hour', '')",
    )
    .bind(CONNECTION)
    .execute(pool)
    .await?;
    sqlx::query("INSERT INTO sessions (id, soundcloud_connection_id) VALUES ($1, $2)")
        .bind(SESSION)
        .bind(CONNECTION)
        .execute(pool)
        .await?;
    Ok(())
}

async fn revoke_behind_the_cache(pool: &PgPool) -> anyhow::Result<()> {
    sqlx::query("DELETE FROM sessions WHERE id = $1")
        .bind(SESSION)
        .execute(pool)
        .await?;
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn a_cached_session_survives_a_revoke_on_another_node_until_the_ttl(
    pool: PgPool,
) -> anyhow::Result<()> {
    seed(&pool).await?;
    let auth = auth(&pool)?;
    assert!(auth.get_session_cached(SESSION).await?.is_some());

    revoke_behind_the_cache(&pool).await?;
    assert!(auth.get_session_cached(SESSION).await?.is_some());

    tokio::time::sleep(Duration::from_millis(5500)).await;
    assert!(auth.get_session_cached(SESSION).await?.is_none());
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn logout_on_this_node_takes_effect_at_once(pool: PgPool) -> anyhow::Result<()> {
    seed(&pool).await?;
    let auth = auth(&pool)?;
    assert!(auth.get_session_cached(SESSION).await?.is_some());

    auth.logout(SESSION).await?;
    assert!(auth.get_session_cached(SESSION).await?.is_none());
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn an_unknown_session_is_never_cached(pool: PgPool) -> anyhow::Result<()> {
    let auth = auth(&pool)?;
    assert!(auth.get_session_cached(SESSION).await?.is_none());
    seed(&pool).await?;
    assert!(auth.get_session_cached(SESSION).await?.is_some());
    Ok(())
}
