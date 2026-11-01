use std::sync::Arc;

use sqlx::PgPool;
use uuid::Uuid;

use crate::error::AppError;
use crate::modules::auth::service::RefreshOutcome;
use crate::modules::auth::{AuthHealthService, AuthService, CoreRpc};
use crate::modules::oauth_apps::OAuthAppsService;
use crate::sc::ScClient;

const APP: Uuid = Uuid::from_u128(0xb1);
const SPARE_APP: Uuid = Uuid::from_u128(0xb2);
const CONNECTION: Uuid = Uuid::from_u128(0xc1);
const SESSION: Uuid = Uuid::from_u128(0xa1);
const LEASE: Uuid = Uuid::from_u128(0xd1);
const GENERATION: i64 = 7;

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

async fn add_app(pool: &PgPool, id: Uuid, active: bool) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO oauth_apps (id, name, client_id, client_secret, redirect_uri, active)
         VALUES ($1, 'test', $1::text, 'secret', 'https://app.test/', $2)",
    )
    .bind(id)
    .bind(active)
    .execute(pool)
    .await?;
    Ok(())
}

async fn connect(
    pool: &PgPool,
    app: Option<Uuid>,
    expires_in_minutes: i64,
    lease: Option<Uuid>,
) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO soundcloud_connections
            (id, soundcloud_user_id, oauth_app_id, access_token, refresh_token, expires_at, scope,
             refresh_generation, refresh_lease_id, refresh_lease_expires_at)
         VALUES ($1, '42', $2, 'access', 'refresh', now() + $3 * interval '1 minute', '',
                 $4, $5, CASE WHEN $5::uuid IS NULL THEN NULL ELSE now() + interval '30 seconds' END)",
    )
    .bind(CONNECTION)
    .bind(app)
    .bind(expires_in_minutes)
    .bind(GENERATION)
    .bind(lease)
    .execute(pool)
    .await?;
    sqlx::query("INSERT INTO sessions (id, soundcloud_connection_id) VALUES ($1, $2)")
        .bind(SESSION)
        .bind(CONNECTION)
        .execute(pool)
        .await?;
    Ok(())
}

async fn untouched(pool: &PgPool) -> anyhow::Result<bool> {
    Ok(sqlx::query_scalar(
        "SELECT refresh_generation = $2
            AND refresh_lease_id IS NULL
            AND last_refresh_attempt_at IS NULL
            AND last_refresh_error_kind IS NULL
            AND retry_at IS NULL
            AND access_token = 'access'
         FROM soundcloud_connections WHERE id = $1",
    )
    .bind(CONNECTION)
    .bind(GENERATION)
    .fetch_one(pool)
    .await?)
}

#[sqlx::test(migrations = "./migrations")]
async fn core_down_gives_a_temporary_error_and_writes_nothing(pool: PgPool) -> anyhow::Result<()> {
    add_app(&pool, APP, true).await?;
    connect(&pool, Some(APP), -60, None).await?;
    let auth = auth(&pool)?;

    let token = auth.get_valid_access_token(SESSION).await;
    let refreshed = auth.refresh_soundcloud(SESSION).await;
    let rejected = auth.refresh_rejected_access_token(SESSION, "access").await;
    let held = auth.mark_access_token_rejected(SESSION, "access").await?;

    assert!(matches!(
        token,
        Err(AppError::SoundCloudTemporarilyUnavailable { .. })
    ));
    assert!(matches!(
        refreshed,
        Err(AppError::SoundCloudTemporarilyUnavailable { .. })
    ));
    assert!(matches!(
        rejected,
        Err(AppError::SoundCloudTemporarilyUnavailable { .. })
    ));
    assert_eq!(held, None);
    assert!(untouched(&pool).await?);
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn a_refresh_running_on_core_is_not_mistaken_for_a_logout(
    pool: PgPool,
) -> anyhow::Result<()> {
    add_app(&pool, APP, true).await?;
    connect(&pool, Some(APP), -60, Some(LEASE)).await?;

    let token = auth(&pool)?.get_valid_access_token(SESSION).await;

    assert!(matches!(
        token,
        Err(AppError::SoundCloudTemporarilyUnavailable { .. })
    ));
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn an_inactive_app_asks_for_a_new_login_instead_of_retrying(
    pool: PgPool,
) -> anyhow::Result<()> {
    add_app(&pool, APP, false).await?;
    add_app(&pool, SPARE_APP, true).await?;
    connect(&pool, Some(APP), -60, None).await?;
    sqlx::query(
        "UPDATE soundcloud_connections
         SET last_refresh_error_kind = 'temporarily_unavailable',
             retry_at = now() + interval '10 minutes'",
    )
    .execute(&pool)
    .await?;
    let auth = auth(&pool)?;

    let token = auth.get_valid_access_token(SESSION).await;
    let refreshed = auth.refresh_soundcloud(SESSION).await?;

    assert!(matches!(
        token,
        Err(AppError::SoundCloudReauthorizationRequired)
    ));
    assert_eq!(refreshed.outcome, RefreshOutcome::ReauthorizationRequired);
    let generation: i64 =
        sqlx::query_scalar("SELECT refresh_generation FROM soundcloud_connections WHERE id = $1")
            .bind(CONNECTION)
            .fetch_one(&pool)
            .await?;
    assert_eq!(generation, GENERATION);
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn a_connection_without_an_app_asks_for_a_new_login(pool: PgPool) -> anyhow::Result<()> {
    add_app(&pool, SPARE_APP, true).await?;
    connect(&pool, None, -60, None).await?;

    let token = auth(&pool)?.get_valid_access_token(SESSION).await;

    assert!(matches!(
        token,
        Err(AppError::SoundCloudReauthorizationRequired)
    ));
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn without_any_active_app_a_new_login_cannot_help(pool: PgPool) -> anyhow::Result<()> {
    add_app(&pool, APP, false).await?;
    connect(&pool, Some(APP), -60, None).await?;
    let auth = auth(&pool)?;

    let token = auth.get_valid_access_token(SESSION).await;
    let refreshed = auth.refresh_soundcloud(SESSION).await;

    assert!(matches!(
        token,
        Err(AppError::SoundCloudTemporarilyUnavailable {
            retry_after_sec: Some(1800)
        })
    ));
    assert!(matches!(
        refreshed,
        Err(AppError::SoundCloudTemporarilyUnavailable {
            retry_after_sec: Some(1800)
        })
    ));
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn a_fresh_token_stays_usable_while_its_app_is_inactive(pool: PgPool) -> anyhow::Result<()> {
    add_app(&pool, APP, false).await?;
    add_app(&pool, SPARE_APP, true).await?;
    connect(&pool, Some(APP), 60, None).await?;
    let auth = auth(&pool)?;

    let token = auth.get_valid_access_token(SESSION).await?;
    let upload_token = auth.get_lasting_access_token(SESSION).await?;
    let refreshed = auth.refresh_soundcloud(SESSION).await?;

    assert_eq!(token, "access");
    assert_eq!(upload_token, "access");
    assert_eq!(refreshed.outcome, RefreshOutcome::AlreadyFresh);
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn an_upload_keeps_a_token_about_to_expire_while_core_is_down(
    pool: PgPool,
) -> anyhow::Result<()> {
    add_app(&pool, APP, true).await?;
    connect(&pool, Some(APP), 2, None).await?;

    let token = auth(&pool)?.get_lasting_access_token(SESSION).await?;

    assert_eq!(token, "access");
    assert!(untouched(&pool).await?);
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn an_upload_asks_for_a_new_login_when_a_token_about_to_expire_has_no_app(
    pool: PgPool,
) -> anyhow::Result<()> {
    add_app(&pool, APP, false).await?;
    add_app(&pool, SPARE_APP, true).await?;
    connect(&pool, Some(APP), 2, None).await?;
    let auth = auth(&pool)?;

    let upload_token = auth.get_lasting_access_token(SESSION).await;
    let token = auth.get_valid_access_token(SESSION).await?;

    assert!(matches!(
        upload_token,
        Err(AppError::SoundCloudReauthorizationRequired)
    ));
    assert_eq!(token, "access");
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn without_any_active_app_an_upload_keeps_a_token_about_to_expire(
    pool: PgPool,
) -> anyhow::Result<()> {
    add_app(&pool, APP, false).await?;
    connect(&pool, Some(APP), 2, None).await?;

    let token = auth(&pool)?.get_lasting_access_token(SESSION).await?;

    assert_eq!(token, "access");
    Ok(())
}
