use std::sync::Arc;

use sqlx::PgPool;

use super::*;
use crate::modules::auth::{AuthHealthService, CoreRpc};
use crate::modules::oauth_apps::OAuthAppsService;
use crate::sc::ScClient;

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
        CoreRpc::unreachable(),
    ))
}

async fn rejected_login(pool: &PgPool, user: &str) -> anyhow::Result<(Uuid, Uuid)> {
    let app = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO oauth_apps (id, name, client_id, client_secret, redirect_uri)
         VALUES ($1, 'test', $1::text, 'secret', 'https://app.test/')",
    )
    .bind(app)
    .execute(pool)
    .await?;
    let connection = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO soundcloud_connections (
             id, soundcloud_user_id, oauth_app_id, access_token, refresh_token, expires_at, scope,
             last_refresh_error_kind, refresh_rejection_count, first_refresh_rejection_at, retry_at
         ) VALUES ($1, $2, $3, 'old-access', 'old-refresh', now() - interval '1 hour', '',
                   'reauthorization_required', 3, now() - interval '1 hour', now() + interval '1 hour')",
    )
    .bind(connection)
    .bind(user)
    .bind(app)
    .execute(pool)
    .await?;
    Ok((app, connection))
}

async fn session(pool: &PgPool, connection: Uuid) -> anyhow::Result<Uuid> {
    let id = Uuid::now_v7();
    sqlx::query("INSERT INTO sessions (id, soundcloud_connection_id) VALUES ($1, $2)")
        .bind(id)
        .bind(connection)
        .execute(pool)
        .await?;
    Ok(id)
}

async fn relogin(
    pool: &PgPool,
    app: Uuid,
    target: Uuid,
    user: &str,
) -> anyhow::Result<StoredConnection> {
    let token = ScTokenResponse {
        access_token: "new-access".to_owned(),
        refresh_token: "new-refresh".to_owned(),
        expires_in: 3600,
        scope: String::new(),
        token_type: String::new(),
    };
    let mut transaction = pool.begin().await?;
    let stored = auth(pool)?
        .store_connection(
            &mut transaction,
            ConnectionCredentials {
                target_session_id: Some(target),
                soundcloud_user_id: user,
                username: Some("listener"),
                oauth_app_id: app,
                token: &token,
                expires_at: Utc::now() + chrono::Duration::hours(1),
            },
        )
        .await?;
    transaction.commit().await?;
    Ok(stored)
}

async fn connection_of(pool: &PgPool, session: Uuid) -> anyhow::Result<Option<Uuid>> {
    Ok(
        sqlx::query_scalar("SELECT soundcloud_connection_id FROM sessions WHERE id = $1")
            .bind(session)
            .fetch_one(pool)
            .await?,
    )
}

#[sqlx::test(migrations = "./migrations")]
async fn a_relogin_renews_the_connection_every_session_of_the_account_uses(
    pool: PgPool,
) -> anyhow::Result<()> {
    let (app, old) = rejected_login(&pool, "42").await?;
    let target = session(&pool, old).await?;
    let linked = session(&pool, old).await?;
    let before: i64 =
        sqlx::query_scalar("SELECT refresh_generation FROM soundcloud_connections WHERE id = $1")
            .bind(old)
            .fetch_one(&pool)
            .await?;

    let stored = relogin(&pool, app, target, "42").await?;

    let renewed = stored.connection;
    assert_eq!(renewed.id, old);
    assert_eq!(stored.orphan_candidate, None);
    assert_eq!(renewed.access_token, "new-access");
    assert!(renewed.last_refresh_error_kind.is_none());
    assert!(renewed.retry_at.is_none());
    assert_eq!(renewed.refresh_generation, before + 1);
    assert_eq!(connection_of(&pool, target).await?, Some(old));
    assert_eq!(connection_of(&pool, linked).await?, Some(old));
    let (refresh_token, login_v, token_v, rows): (String, i64, i64, i64) = sqlx::query_as(
        "SELECT refresh_token, login_v, token_v, (SELECT count(*) FROM soundcloud_connections)
         FROM soundcloud_connections WHERE id = $1",
    )
    .bind(old)
    .fetch_one(&pool)
    .await?;
    assert_eq!(refresh_token, "new-refresh");
    assert!(login_v > 0);
    assert_eq!(login_v, token_v);
    assert_eq!(rows, 1);
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn a_login_as_another_account_gets_its_own_connection(pool: PgPool) -> anyhow::Result<()> {
    let (app, old) = rejected_login(&pool, "42").await?;
    let target = session(&pool, old).await?;
    let linked = session(&pool, old).await?;

    let stored = relogin(&pool, app, target, "77").await?;

    assert_ne!(stored.connection.id, old);
    assert_eq!(stored.orphan_candidate, Some(old));
    assert_eq!(connection_of(&pool, linked).await?, Some(old));
    Ok(())
}
