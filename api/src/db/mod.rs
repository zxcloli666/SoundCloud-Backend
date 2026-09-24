use std::str::FromStr;
use std::time::Duration;

use anyhow::{Context, ensure};
use sqlx::postgres::{PgConnectOptions, PgPoolOptions, PgSslMode};
use sqlx::{ConnectOptions, PgPool};
use tracing::log::LevelFilter;

use crate::config::{AppConfig, DatabaseCfg};

const REQUIRED_CORE_SCHEMA_VERSION: i64 = 109;

fn connect_opts(cfg: &DatabaseCfg) -> Result<PgConnectOptions, sqlx::Error> {
    let mut opts = PgConnectOptions::from_str(&cfg.url)?;

    if let Some(mode) = &cfg.ssl.mode {
        opts = opts.ssl_mode(parse_ssl_mode(mode)?);
    }
    if let Some(ca) = &cfg.ssl.root_cert {
        opts = opts.ssl_root_cert(ca);
    }
    if let Some(cert) = &cfg.ssl.client_cert {
        opts = opts.ssl_client_cert(cert);
    }
    if let Some(key) = &cfg.ssl.client_key {
        opts = opts.ssl_client_key(key);
    }
    if cfg.ssl.mode.is_none() && cfg.ssl.root_cert.is_some() {
        opts = opts.ssl_mode(PgSslMode::VerifyFull);
    }

    Ok(opts
        .log_statements(LevelFilter::Debug)
        .log_slow_statements(LevelFilter::Warn, Duration::from_millis(500)))
}

fn parse_ssl_mode(raw: &str) -> Result<PgSslMode, sqlx::Error> {
    PgSslMode::from_str(raw).map_err(|_| {
        sqlx::Error::Configuration(
            format!(
                "invalid DATABASE_SSL_MODE {raw:?} \
                 (disable|allow|prefer|require|verify-ca|verify-full)"
            )
            .into(),
        )
    })
}

async fn pool(cfg: &DatabaseCfg) -> Result<PgPool, sqlx::Error> {
    PgPoolOptions::new()
        .max_connections(cfg.pool_max)
        .acquire_timeout(cfg.acquire_timeout)
        .idle_timeout(Some(Duration::from_secs(600)))
        .max_lifetime(Some(Duration::from_secs(1800)))
        .test_before_acquire(true)
        .connect_with(connect_opts(cfg)?)
        .await
}

pub async fn connect(cfg: &AppConfig) -> Result<PgPool, sqlx::Error> {
    pool(&cfg.database).await
}

pub async fn verify_schema(pool: &PgPool) -> anyhow::Result<()> {
    let applied =
        sqlx::query_scalar::<_, bool>("SELECT success FROM _sqlx_migrations WHERE version = $1")
            .bind(REQUIRED_CORE_SCHEMA_VERSION)
            .fetch_optional(pool)
            .await
            .context("core migration history is unavailable")?;
    ensure!(
        applied == Some(true),
        "core schema is outdated: migration {REQUIRED_CORE_SCHEMA_VERSION} is required"
    );
    let playlist_shadow_ready = sqlx::query_scalar::<_, bool>(
        "SELECT to_regclass('playlist_track_projection') IS NOT NULL
             AND to_regclass('playlist_membership_state') IS NOT NULL
             AND to_regclass('playlist_remote_observations') IS NOT NULL
             AND to_regclass('playlist_membership_state_reconcile_due_idx') IS NOT NULL
             AND NOT EXISTS (
                 SELECT 1
                 FROM information_schema.columns
                 WHERE table_schema = current_schema()
                   AND table_name = 'playlists'
                   AND column_name IN ('desired_rev', 'synced_rev', 'tracks_synced_at')
             )",
    )
    .fetch_one(pool)
    .await
    .context("playlist shadow schema validation failed")?;
    ensure!(
        playlist_shadow_ready,
        "playlist shadow schema is incomplete; apply migrations through 0087"
    );
    Ok(())
}

#[cfg(test)]
mod connection_discipline_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::DbSslCfg;

    fn cfg(url: &str, ssl: DbSslCfg) -> DatabaseCfg {
        DatabaseCfg {
            url: url.to_string(),
            ssl,
            pool_max: 1,
            acquire_timeout: Duration::from_secs(1),
        }
    }

    #[test]
    fn ssl_env_overrides_url_query() {
        let opts = connect_opts(&cfg(
            "postgres://u:p@h:5432/db?sslmode=disable",
            DbSslCfg {
                mode: Some("verify-full".into()),
                root_cert: Some("/pg-certs/ca.crt".into()),
                client_cert: Some("/pg-certs/client.crt".into()),
                client_key: Some("/pg-certs/client.key".into()),
            },
        ))
        .expect("opts");
        assert!(matches!(opts.get_ssl_mode(), PgSslMode::VerifyFull));
    }

    #[test]
    fn ssl_mode_defaults_to_verify_full_when_ca_given() {
        let opts = connect_opts(&cfg(
            "postgres://u:p@h:5432/db",
            DbSslCfg {
                root_cert: Some("/pg-certs/ca.crt".into()),
                ..Default::default()
            },
        ))
        .expect("opts");
        assert!(matches!(opts.get_ssl_mode(), PgSslMode::VerifyFull));
    }

    #[test]
    fn url_ssl_params_survive_when_no_env_override() {
        let opts = connect_opts(&cfg(
            "postgres://u:p@h:5432/db?sslmode=require",
            DbSslCfg::default(),
        ))
        .expect("opts");
        assert!(matches!(opts.get_ssl_mode(), PgSslMode::Require));
    }

    #[test]
    fn bad_ssl_mode_is_a_config_error() {
        let err = connect_opts(&cfg(
            "postgres://u:p@h:5432/db",
            DbSslCfg {
                mode: Some("verify-most".into()),
                ..Default::default()
            },
        ))
        .expect_err("should reject");
        assert!(err.to_string().contains("verify-most"), "{err}");
    }

    #[sqlx::test(migrations = false)]
    async fn schema_check_requires_latest_core_migration(pool: PgPool) -> anyhow::Result<()> {
        sqlx::query(
            "CREATE TABLE _sqlx_migrations (
                version bigint PRIMARY KEY,
                success boolean NOT NULL
            )",
        )
        .execute(&pool)
        .await?;

        let error = verify_schema(&pool).await.expect_err("outdated schema");
        assert!(
            error
                .to_string()
                .contains(&format!("migration {REQUIRED_CORE_SCHEMA_VERSION}"))
        );

        sqlx::query("INSERT INTO _sqlx_migrations (version, success) VALUES ($1, true)")
            .bind(REQUIRED_CORE_SCHEMA_VERSION)
            .execute(&pool)
            .await?;
        sqlx::raw_sql(
            "CREATE TABLE playlists (urn text PRIMARY KEY);
             CREATE TABLE playlist_track_projection (playlist_urn text);
             CREATE TABLE playlist_membership_state (
                 playlist_urn text PRIMARY KEY,
                 next_reconcile_at timestamptz,
                 sync_status text NOT NULL
             );
             CREATE TABLE playlist_remote_observations (id uuid PRIMARY KEY);
             CREATE INDEX playlist_membership_state_reconcile_due_idx
                 ON playlist_membership_state (next_reconcile_at, playlist_urn)
                 WHERE sync_status <> 'clean';",
        )
        .execute(&pool)
        .await?;

        verify_schema(&pool).await
    }
}
