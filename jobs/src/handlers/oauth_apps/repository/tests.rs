use std::time::Duration;

use sqlx::PgPool;
use uuid::Uuid;

use super::*;
use crate::config::OAuthAppBootstrap;
use crate::handlers::oauth_apps::model::{ClaimedApp, Token};

async fn install_schema(pool: &PgPool) -> anyhow::Result<()> {
    sqlx::raw_sql(
        "CREATE EXTENSION IF NOT EXISTS pgcrypto;
         CREATE TABLE oauth_apps (
             id uuid PRIMARY KEY,
             name text NOT NULL,
             client_id text NOT NULL,
             client_secret text NOT NULL,
             redirect_uri text NOT NULL,
             active boolean NOT NULL DEFAULT true,
             updated_at timestamptz NOT NULL DEFAULT now()
         );
         CREATE TABLE oauth_app_tokens (
             oauth_app_id uuid PRIMARY KEY REFERENCES oauth_apps(id) ON DELETE CASCADE,
             access_token text,
             scope text,
             expires_at timestamptz NOT NULL,
             refreshed_at timestamptz NOT NULL DEFAULT now(),
             refresh_attempts integer NOT NULL DEFAULT 0,
             last_refresh_error text,
             refresh_token text,
             generation uuid NOT NULL DEFAULT gen_random_uuid()
         );",
    )
    .execute(pool)
    .await?;
    sqlx::raw_sql(include_str!(
        "../../../../../api/migrations/0063_oauth_token_coordination.sql"
    ))
    .execute(pool)
    .await?;
    Ok(())
}

async fn insert_app(pool: &PgPool, client_id: &str) -> anyhow::Result<ClaimedApp> {
    let id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO oauth_apps (id, name, client_id, client_secret, redirect_uri)
         VALUES ($1, 'test', $2, 'secret', 'https://localhost/callback')",
    )
    .bind(id)
    .bind(client_id)
    .execute(pool)
    .await?;
    Ok(ClaimedApp {
        id,
        client_id: client_id.to_owned(),
        client_secret: "secret".to_owned(),
        refresh_token: None,
        refresh_attempts: None,
        lease_id: Uuid::now_v7(),
    })
}

#[sqlx::test(migrations = false)]
async fn configured_app_bootstrap_is_single_winner(pool: PgPool) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    let first = OAuthRefreshRepository::new(pool.clone());
    let second = OAuthRefreshRepository::new(pool.clone());
    let app = OAuthAppBootstrap {
        name: "default".to_owned(),
        client_id: " configured-client ".to_owned(),
        client_secret: "secret".to_owned().into(),
        redirect_uri: "https://localhost/callback".to_owned(),
    };

    let (left, right) = tokio::join!(first.bootstrap_app(&app), second.bootstrap_app(&app));
    let left = left?;
    let right = right?;
    let stored: (i64, String) = sqlx::query_as("SELECT count(*), min(client_id) FROM oauth_apps")
        .fetch_one(&pool)
        .await?;

    assert_eq!(left, right);
    assert_eq!(stored, (1, "configured-client".to_owned()));
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn configured_app_is_inserted_when_other_apps_already_exist(
    pool: PgPool,
) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    insert_app(&pool, "other-client").await?;
    let configured = OAuthAppBootstrap {
        name: "default".to_owned(),
        client_id: "configured-client".to_owned(),
        client_secret: "secret".to_owned().into(),
        redirect_uri: "https://localhost/callback".to_owned(),
    };

    let app_id = OAuthRefreshRepository::new(pool.clone())
        .bootstrap_app(&configured)
        .await?;

    let stored: String = sqlx::query_scalar("SELECT client_id FROM oauth_apps WHERE id = $1")
        .bind(app_id)
        .fetch_one(&pool)
        .await?;
    assert_eq!(stored, "configured-client");
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn configured_app_reconciles_existing_credentials(pool: PgPool) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    let existing = insert_app(&pool, " configured-client ").await?;
    sqlx::query(
        "UPDATE oauth_apps
         SET name = 'old',
             client_secret = 'old-secret',
             redirect_uri = 'https://old.invalid/callback',
             active = false
         WHERE id = $1",
    )
    .bind(existing.id)
    .execute(&pool)
    .await?;
    let configured = OAuthAppBootstrap {
        name: "default".to_owned(),
        client_id: "configured-client".to_owned(),
        client_secret: "new-secret".to_owned().into(),
        redirect_uri: "https://localhost/callback".to_owned(),
    };

    let app_id = OAuthRefreshRepository::new(pool.clone())
        .bootstrap_app(&configured)
        .await?;
    let stored = sqlx::query_as::<_, (String, String, String, String, bool)>(
        "SELECT name, client_id, client_secret, redirect_uri, active
         FROM oauth_apps
         WHERE id = $1",
    )
    .bind(app_id)
    .fetch_one(&pool)
    .await?;

    assert_eq!(app_id, existing.id);
    assert_eq!(stored.0, "default");
    assert_eq!(stored.1, "configured-client");
    assert_eq!(stored.2, "new-secret");
    assert_eq!(stored.3, "https://localhost/callback");
    assert!(stored.4);
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn only_one_replica_claims_an_expired_app(pool: PgPool) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    let app = insert_app(&pool, "client-one").await?;
    sqlx::query(
        "INSERT INTO oauth_app_tokens (oauth_app_id, expires_at)
         VALUES ($1, now() - interval '1 minute')",
    )
    .bind(app.id)
    .execute(&pool)
    .await?;
    let repository = OAuthRefreshRepository::new(pool);
    let (first, second) = tokio::join!(
        repository.claim_due(1, Duration::from_secs(60)),
        repository.claim_due(1, Duration::from_secs(60))
    );

    assert_eq!(first?.len() + second?.len(), 1);
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn client_identity_cap_covers_duplicate_oauth_app_rows(pool: PgPool) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    let app = insert_app(&pool, "shared-client").await?;
    let duplicate = insert_app(&pool, "\u{3000}shared-client\u{00a0}").await?;
    seed_reservations(&pool, &app, 45, "2 hours").await?;
    let repository = OAuthRefreshRepository::new(pool);

    assert!(
        repository
            .reserve_client_credentials(&duplicate)
            .await?
            .reservation_id
            .is_none()
    );
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn deleting_an_app_does_not_reset_its_issuance_cap(pool: PgPool) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    let app = insert_app(&pool, "shared-client").await?;
    seed_reservations(&pool, &app, 45, "2 hours").await?;
    sqlx::query("DELETE FROM oauth_apps WHERE id = $1")
        .bind(app.id)
        .execute(&pool)
        .await?;
    let replacement = insert_app(&pool, "shared-client").await?;
    let repository = OAuthRefreshRepository::new(pool);

    assert!(
        repository
            .reserve_client_credentials(&replacement)
            .await?
            .reservation_id
            .is_none()
    );
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn egress_cap_blocks_a_different_oauth_identity(pool: PgPool) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    let app = insert_app(&pool, "client-one").await?;
    let next = insert_app(&pool, "client-two").await?;
    seed_reservations(&pool, &app, 27, "0 seconds").await?;
    let repository = OAuthRefreshRepository::new(pool);

    assert!(
        repository
            .reserve_client_credentials(&next)
            .await?
            .reservation_id
            .is_none()
    );
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn definitive_rejection_releases_only_its_reservation(pool: PgPool) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    let app = insert_app(&pool, "client-one").await?;
    let repository = OAuthRefreshRepository::new(pool.clone());
    let reservation = repository.reserve_client_credentials(&app).await?;
    let reservation_id = reservation
        .reservation_id
        .ok_or_else(|| anyhow::anyhow!("reservation missing"))?;

    repository.release_reservation(reservation_id).await?;
    let retained: i64 = sqlx::query_scalar(
        "SELECT count(*)
         FROM oauth_app_token_issuance_reservations
         WHERE id = $1 AND released_at IS NULL",
    )
    .bind(reservation_id)
    .fetch_one(&pool)
    .await?;

    assert_eq!(retained, 0);
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn ambiguous_client_credentials_attempt_retains_its_reservation(
    pool: PgPool,
) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    let app = insert_app(&pool, "client-one").await?;
    let repository = OAuthRefreshRepository::new(pool.clone());
    let reservation = repository.reserve_client_credentials(&app).await?;
    let reservation_id = reservation
        .reservation_id
        .ok_or_else(|| anyhow::anyhow!("reservation missing"))?;
    let retained: i64 = sqlx::query_scalar(
        "SELECT count(*)
         FROM oauth_app_token_issuance_reservations
         WHERE id = $1 AND released_at IS NULL",
    )
    .bind(reservation_id)
    .fetch_one(&pool)
    .await?;

    assert_eq!(retained, 1);
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn client_credentials_fallback_clears_a_single_use_refresh_token(
    pool: PgPool,
) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    let mut app = insert_app(&pool, "client-one").await?;
    sqlx::query(
        "INSERT INTO oauth_app_tokens (oauth_app_id, refresh_token, expires_at)
         VALUES ($1, 'single-use-refresh', now() - interval '1 minute')",
    )
    .bind(app.id)
    .execute(&pool)
    .await?;
    app.lease_id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO oauth_app_token_refresh_state (
             oauth_app_id, lease_id, lease_expires_at
         )
         VALUES ($1, $2, now() + interval '1 minute')",
    )
    .bind(app.id)
    .bind(app.lease_id)
    .execute(&pool)
    .await?;
    let repository = OAuthRefreshRepository::new(pool.clone());
    let token = Token {
        access_token: "new-access".to_owned(),
        refresh_token: None,
        scope: None,
        expires_in: 3600,
    };

    assert!(repository.complete(&app, &token).await?);
    let refresh_token: Option<String> =
        sqlx::query_scalar("SELECT refresh_token FROM oauth_app_tokens WHERE oauth_app_id = $1")
            .bind(app.id)
            .fetch_one(&pool)
            .await?;
    assert_eq!(refresh_token, None);
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn completed_refresh_rotates_token_generation(pool: PgPool) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    let mut app = insert_app(&pool, "client-one").await?;
    sqlx::query(
        "INSERT INTO oauth_app_tokens (oauth_app_id, access_token, expires_at)
         VALUES ($1, 'old-access', now() - interval '1 minute')",
    )
    .bind(app.id)
    .execute(&pool)
    .await?;
    let old_generation: Uuid =
        sqlx::query_scalar("SELECT generation FROM oauth_app_tokens WHERE oauth_app_id = $1")
            .bind(app.id)
            .fetch_one(&pool)
            .await?;
    app.lease_id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO oauth_app_token_refresh_state (
             oauth_app_id, lease_id, lease_expires_at
         ) VALUES ($1, $2, now() + interval '1 minute')",
    )
    .bind(app.id)
    .bind(app.lease_id)
    .execute(&pool)
    .await?;
    let token = Token {
        access_token: "new-access".to_owned(),
        refresh_token: Some("new-refresh".to_owned()),
        scope: None,
        expires_in: 3600,
    };

    assert!(
        OAuthRefreshRepository::new(pool.clone())
            .complete(&app, &token)
            .await?
    );
    let new_generation: Uuid =
        sqlx::query_scalar("SELECT generation FROM oauth_app_tokens WHERE oauth_app_id = $1")
            .bind(app.id)
            .fetch_one(&pool)
            .await?;

    assert_ne!(new_generation, old_generation);
    Ok(())
}

async fn seed_reservations(
    pool: &PgPool,
    app: &ClaimedApp,
    count: i32,
    age: &str,
) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO oauth_app_token_issuance_reservations (
             id, oauth_app_id, client_id, reserved_at
         )
         SELECT gen_random_uuid(), $1, $2, now() - $3::interval
         FROM generate_series(1, $4)",
    )
    .bind(app.id)
    .bind(&app.client_id)
    .bind(age)
    .bind(count)
    .execute(pool)
    .await?;
    Ok(())
}
