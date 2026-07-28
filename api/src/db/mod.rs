use std::str::FromStr;
use std::time::Duration;

use sqlx::postgres::{PgConnectOptions, PgPoolOptions, PgSslMode};
use sqlx::{ConnectOptions, PgPool};
use tracing::log::LevelFilter;

use crate::config::{AppConfig, DatabaseCfg};

pub mod advisory_locks;

/// Ops-БД: телеметрия/обучающие выборки и прочее, что не нужно отдаче.
///
/// Живёт на кроновой ноде (load) отдельным постгресом. На main/star не
/// сконфигурирована — тогда это `None`, и все её потребители тихо становятся
/// no-op'ами: ни заглушечной БД, ни падения на старте.
#[derive(Clone, Default)]
pub struct OpsDb(Option<PgPool>);

impl OpsDb {
    pub fn disabled() -> Self {
        Self(None)
    }

    pub fn from_pool(pool: PgPool) -> Self {
        Self(Some(pool))
    }

    /// `None` — ops-БД не подключена; вызывающий обязан тихо выйти.
    pub fn pool(&self) -> Option<&PgPool> {
        self.0.as_ref()
    }

    pub fn is_enabled(&self) -> bool {
        self.0.is_some()
    }
}

/// Разбирает URL и накладывает сверху TLS/mTLS из отдельных env-переменных.
/// Что задано отдельно — выигрывает у того, что зашито в query-строку URL.
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
    // Дали серты, но не сказали режим — по умолчанию проверяем цепочку и хост.
    // Иначе mTLS-конфиг молча деградировал бы до `prefer` (дефолт sqlx).
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

/// `Ok(OpsDb::disabled())`, если ops-БД не сконфигурирована.
pub async fn connect_ops(cfg: &AppConfig) -> Result<OpsDb, sqlx::Error> {
    match &cfg.ops_database {
        Some(ops) => Ok(OpsDb::from_pool(pool(ops).await?)),
        None => Ok(OpsDb::disabled()),
    }
}

pub async fn migrate(pool: &PgPool) -> Result<(), sqlx::Error> {
    run_migrations(pool, advisory_locks::MIGRATIONS, &CORE_MIGRATOR).await
}

pub async fn migrate_ops(pool: &PgPool) -> Result<(), sqlx::Error> {
    run_migrations(pool, advisory_locks::MIGRATIONS_OPS, &OPS_MIGRATOR).await
}

/// Наборы миграций пронумерованы в непересекающихся диапазонах (core `0000+`,
/// ops `9000+`), поэтому обе цепочки могут ужиться в ОДНОЙ базе на общем
/// `_sqlx_migrations` — это дефолт для локалки и валидный all-in-one деплой.
/// Ради этого нужен `ignore_missing`: иначе core-мигратор увидит применённые
/// `9000+` (и наоборот) и упадёт `VersionMissing`. Проверка контрольных сумм
/// (`VersionMismatch` на правку применённой миграции) при этом остаётся.
static CORE_MIGRATOR: sqlx::migrate::Migrator = {
    let mut m = sqlx::migrate!("./migrations");
    m.ignore_missing = true;
    m
};

static OPS_MIGRATOR: sqlx::migrate::Migrator = {
    let mut m = sqlx::migrate!("./migrations-ops");
    m.ignore_missing = true;
    m
};

async fn run_migrations(
    pool: &PgPool,
    lock: i64,
    migrator: &sqlx::migrate::Migrator,
) -> Result<(), sqlx::Error> {
    let mut conn = pool.acquire().await?;
    sqlx::query("SELECT pg_advisory_lock($1)")
        .bind(lock)
        .execute(&mut *conn)
        .await?;

    let result = migrator.run(&mut *conn).await;

    sqlx::query("SELECT pg_advisory_unlock($1)")
        .bind(lock)
        .execute(&mut *conn)
        .await?;

    result.map_err(|e| sqlx::Error::Migrate(Box::new(e)))
}

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
}
