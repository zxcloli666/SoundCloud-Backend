use std::str::FromStr;
use std::sync::LazyLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use anyhow::{Context, ensure};
use sqlx::postgres::{PgConnectOptions, PgPoolOptions, PgSslMode};
use sqlx::{ConnectOptions, Connection, PgConnection, PgPool};
use tracing::log::LevelFilter;

use crate::config::{AppConfig, DatabaseCfg};

mod retry;

pub use retry::retry_conflicts;

const REQUIRED_CORE_SCHEMA_VERSION: i64 = 295;
const RECOMMENDATIONS_STATEMENT_TIMEOUT: &str = "15s";
const RELEASE_PING_TIMEOUT: Duration = Duration::from_secs(5);

static CLOCK: LazyLock<Instant> = LazyLock::new(Instant::now);
static CORE_RELEASED_AT_MS: AtomicU64 = AtomicU64::new(0);

fn clock_ms() -> u64 {
    CLOCK.elapsed().as_millis() as u64
}

pub fn core_pool_quiet_for() -> Duration {
    let released = CORE_RELEASED_AT_MS.load(Ordering::Relaxed);
    Duration::from_millis(clock_ms().saturating_sub(released))
}

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
        .options([("jit", "off"), ("scd.writer", "api")])
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

fn recommendations_share(pool_max: u32) -> u32 {
    (pool_max / 4).max(1)
}

fn core_share(pool_max: u32) -> u32 {
    pool_max
        .saturating_sub(recommendations_share(pool_max))
        .max(1)
}

fn pool_options(
    cfg: &DatabaseCfg,
    max_connections: u32,
    released_at: Option<&'static AtomicU64>,
) -> PgPoolOptions {
    PgPoolOptions::new()
        .max_connections(max_connections)
        .acquire_timeout(cfg.acquire_timeout)
        .idle_timeout(Some(Duration::from_secs(600)))
        .max_lifetime(Some(Duration::from_secs(1800)))
        .test_before_acquire(true)
        .after_release(move |connection, _| Box::pin(settle_released(connection, released_at)))
}

async fn settle_released(
    connection: &mut PgConnection,
    released_at: Option<&'static AtomicU64>,
) -> Result<bool, sqlx::Error> {
    match tokio::time::timeout(RELEASE_PING_TIMEOUT, connection.ping()).await {
        Ok(Ok(())) => {
            if let Some(released_at) = released_at {
                released_at.store(clock_ms(), Ordering::Relaxed);
            }
            Ok(true)
        }
        Ok(Err(error)) => Err(error),
        Err(_) => Err(sqlx::Error::Io(std::io::Error::new(
            std::io::ErrorKind::TimedOut,
            "released connection did not settle",
        ))),
    }
}

async fn pool(
    cfg: &DatabaseCfg,
    max_connections: u32,
    opts: PgConnectOptions,
    released_at: Option<&'static AtomicU64>,
) -> Result<PgPool, sqlx::Error> {
    pool_options(cfg, max_connections, released_at)
        .connect_with(opts)
        .await
}

pub async fn connect(cfg: &AppConfig) -> Result<PgPool, sqlx::Error> {
    let db = &cfg.database;
    pool(
        db,
        core_share(db.pool_max),
        connect_opts(db)?,
        Some(&CORE_RELEASED_AT_MS),
    )
    .await
}

pub async fn connect_recommendations(cfg: &AppConfig) -> Result<PgPool, sqlx::Error> {
    let db = &cfg.database;
    let opts =
        connect_opts(db)?.options([("statement_timeout", RECOMMENDATIONS_STATEMENT_TIMEOUT)]);
    pool(db, recommendations_share(db.pool_max), opts, None).await
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
    fn recommendations_take_a_quarter_of_the_pool() {
        assert_eq!((core_share(20), recommendations_share(20)), (15, 5));
        assert_eq!((core_share(10), recommendations_share(10)), (8, 2));
        assert_eq!((core_share(1), recommendations_share(1)), (1, 1));
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

    #[sqlx::test(migrations = false)]
    async fn a_connection_dropped_mid_query_frees_its_pool_slot(
        _: PgPoolOptions,
        opts: PgConnectOptions,
    ) -> anyhow::Result<()> {
        let mut single = cfg("postgres://u:p@h:5432/db", DbSslCfg::default());
        single.acquire_timeout = Duration::from_secs(60);
        let pool = pool_options(&single, 1, None).connect_with(opts).await?;

        let mut tx = pool.begin().await?;
        let abandoned = tokio::time::timeout(
            Duration::from_millis(300),
            sqlx::query("SELECT pg_sleep(60)").execute(&mut *tx),
        )
        .await;
        assert!(abandoned.is_err());
        drop(tx);

        let reacquired = tokio::time::timeout(
            RELEASE_PING_TIMEOUT + Duration::from_secs(5),
            pool.acquire(),
        )
        .await;
        let mut connection = reacquired.expect("the pool slot stayed taken")?;
        sqlx::query(
            "SELECT pg_terminate_backend(pid) FROM pg_stat_activity
             WHERE query = 'SELECT pg_sleep(60)' AND pid <> pg_backend_pid()",
        )
        .execute(&mut *connection)
        .await?;
        Ok(())
    }

    #[tokio::test]
    async fn pools_run_with_jit_off_and_recommendations_keep_their_timeout() -> anyhow::Result<()> {
        let url = std::env::var("DATABASE_URL")?;
        let mut cfg = cfg(&url, DbSslCfg::default());
        cfg.acquire_timeout = Duration::from_secs(20);
        let core = pool(&cfg, 1, connect_opts(&cfg)?, None).await?;
        let recommendations = pool(
            &cfg,
            1,
            connect_opts(&cfg)?.options([("statement_timeout", RECOMMENDATIONS_STATEMENT_TIMEOUT)]),
            None,
        )
        .await?;

        let core_jit: String = sqlx::query_scalar("SHOW jit").fetch_one(&core).await?;
        let core_timeout: String = sqlx::query_scalar("SHOW statement_timeout")
            .fetch_one(&core)
            .await?;
        let core_writer: String = sqlx::query_scalar("SHOW scd.writer")
            .fetch_one(&core)
            .await?;
        let recommendations_jit: String = sqlx::query_scalar("SHOW jit")
            .fetch_one(&recommendations)
            .await?;
        let recommendations_timeout: String = sqlx::query_scalar("SHOW statement_timeout")
            .fetch_one(&recommendations)
            .await?;

        assert_eq!(core_jit, "off");
        assert_eq!(core_writer, "api");
        assert_ne!(core_timeout, "15s");
        assert_eq!(recommendations_jit, "off");
        assert_eq!(recommendations_timeout, "15s");
        Ok(())
    }
}
