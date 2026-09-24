use std::fmt::{Display, Formatter};
use std::str::FromStr;
use std::time::Duration;

use sqlx::postgres::{PgConnectOptions, PgPoolOptions, PgSslMode};
use sqlx::{ConnectOptions, PgPool};
use tracing::log::LevelFilter;

use crate::config::{DatabaseConfig, PoolConfig, SessionLimits};

#[derive(Debug)]
pub struct DatabaseError {
    stage: &'static str,
    source: sqlx::Error,
}

impl Display for DatabaseError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "database {} failed: {}", self.stage, self.source)
    }
}

impl std::error::Error for DatabaseError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.source)
    }
}

pub async fn connect(
    database: &DatabaseConfig,
    pool: &PoolConfig,
    application_name: &str,
) -> Result<PgPool, DatabaseError> {
    validate_pool(pool)?;
    let options = connect_options(database, application_name)?;
    let limits = pool.session.clone();

    PgPoolOptions::new()
        .min_connections(pool.minimum)
        .max_connections(pool.maximum)
        .acquire_timeout(pool.acquire_timeout)
        .idle_timeout(Some(Duration::from_secs(600)))
        .max_lifetime(Some(Duration::from_secs(1_800)))
        .test_before_acquire(false)
        .after_connect(move |connection, _metadata| {
            let limits = limits.clone();
            Box::pin(async move { apply_session_limits(connection, &limits).await })
        })
        .connect_with(options)
        .await
        .map_err(|source| DatabaseError {
            stage: "connection",
            source,
        })
}

fn validate_pool(pool: &PoolConfig) -> Result<(), DatabaseError> {
    if pool.maximum > 0 && pool.minimum <= pool.maximum {
        return Ok(());
    }

    Err(DatabaseError {
        stage: "configuration",
        source: sqlx::Error::Configuration(
            format!(
                "minimum pool size {} exceeds maximum {}",
                pool.minimum, pool.maximum
            )
            .into(),
        ),
    })
}

fn connect_options(
    database: &DatabaseConfig,
    application_name: &str,
) -> Result<PgConnectOptions, DatabaseError> {
    let mut options =
        PgConnectOptions::from_str(&database.url).map_err(|source| DatabaseError {
            stage: "URL parsing",
            source,
        })?;

    if let Some(mode) = &database.tls.mode {
        options = options.ssl_mode(parse_ssl_mode(mode)?);
    }
    if let Some(path) = &database.tls.root_certificate {
        options = options.ssl_root_cert(path);
    }
    if let Some(path) = &database.tls.client_certificate {
        options = options.ssl_client_cert(path);
    }
    if let Some(path) = &database.tls.client_key {
        options = options.ssl_client_key(path);
    }

    Ok(options
        .application_name(application_name)
        .log_statements(LevelFilter::Debug)
        .log_slow_statements(LevelFilter::Warn, Duration::from_millis(500)))
}

fn parse_ssl_mode(value: &str) -> Result<PgSslMode, DatabaseError> {
    PgSslMode::from_str(value).map_err(|_| DatabaseError {
        stage: "TLS configuration",
        source: sqlx::Error::Configuration(
            format!(
                "unsupported SSL mode {value:?}; expected disable, allow, prefer, require, verify-ca, or verify-full"
            )
            .into(),
        ),
    })
}

async fn apply_session_limits(
    connection: &mut sqlx::PgConnection,
    limits: &SessionLimits,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "SELECT
             set_config('statement_timeout', $1, false),
             set_config('lock_timeout', $2, false),
             set_config('idle_in_transaction_session_timeout', $3, false)",
    )
    .bind(duration_setting(limits.statement_timeout))
    .bind(duration_setting(limits.lock_timeout))
    .bind(duration_setting(limits.idle_transaction_timeout))
    .execute(connection)
    .await?;
    Ok(())
}

fn duration_setting(duration: Duration) -> String {
    format!("{}ms", duration.as_millis())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::database::TlsConfig;

    const CANCELLED: &str = "57014";
    const POOL_OWNER: &str = "src/db/pool.rs";
    const BUILDER: &str = "PgPoolOptions::new()";

    #[test]
    fn session_duration_uses_postgres_milliseconds() {
        assert_eq!(duration_setting(Duration::from_millis(1_250)), "1250ms");
    }

    fn sources() -> Vec<(String, String)> {
        fn walk(directory: &std::path::Path, found: &mut Vec<std::path::PathBuf>) {
            for entry in std::fs::read_dir(directory).expect("the source directory is readable") {
                let path = entry.expect("the directory entry is readable").path();
                if path.is_dir() {
                    walk(&path, found);
                } else if path.extension().is_some_and(|extension| extension == "rs") {
                    found.push(path);
                }
            }
        }
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let mut paths = Vec::new();
        walk(&root.join("src"), &mut paths);
        paths
            .into_iter()
            .map(|path| {
                let shown = path
                    .strip_prefix(&root)
                    .expect("every source lives under the crate root")
                    .to_string_lossy()
                    .into_owned();
                let body = std::fs::read_to_string(&path).expect("every source is readable");
                (shown, body)
            })
            .collect()
    }

    #[test]
    fn a_pool_that_serves_a_job_is_built_in_exactly_one_place() {
        let mut owner_builds_one = false;
        for (path, body) in sources() {
            let Some(at) = body.find(BUILDER) else {
                continue;
            };
            if path == POOL_OWNER {
                owner_builds_one = true;
                continue;
            }
            if path.ends_with("tests.rs") {
                continue;
            }
            let cfg_test = body.find("#[cfg(test)]").unwrap_or(usize::MAX);
            assert!(
                at > cfg_test,
                "{path} builds its own pool outside the tests; a pool built anywhere but \
                 {POOL_OWNER} arrives without statement, lock and idle budgets, and one stuck \
                 query then holds a connection and its locks for as long as it likes"
            );
        }
        assert!(
            owner_builds_one,
            "{POOL_OWNER} no longer builds the pool; point this guard at whatever does"
        );
    }

    fn bounded(statement: Duration) -> (DatabaseConfig, PoolConfig) {
        let session = SessionLimits {
            statement_timeout: statement,
            lock_timeout: Duration::from_millis(250),
            idle_transaction_timeout: Duration::from_millis(750),
        };
        let pool = PoolConfig {
            minimum: 0,
            maximum: 1,
            acquire_timeout: Duration::from_secs(5),
            session,
        };
        let database = DatabaseConfig {
            url: std::env::var("DATABASE_URL").expect("the build database is configured"),
            tls: TlsConfig {
                mode: None,
                root_certificate: None,
                client_certificate: None,
                client_key: None,
                require_mtls: false,
            },
            fast_pool: pool.clone(),
            bulk_pool: pool.clone(),
        };
        (database, pool)
    }

    #[tokio::test]
    #[ignore = "requires a live PostgreSQL"]
    async fn every_connection_arrives_with_the_limits_already_set() -> anyhow::Result<()> {
        let (database, pool) = bounded(Duration::from_secs(7));
        let pool = connect(&database, &pool, "jobs-limits-test").await?;

        let statement: String = sqlx::query_scalar("SHOW statement_timeout")
            .fetch_one(&pool)
            .await?;
        let lock: String = sqlx::query_scalar("SHOW lock_timeout")
            .fetch_one(&pool)
            .await?;
        let idle: String = sqlx::query_scalar("SHOW idle_in_transaction_session_timeout")
            .fetch_one(&pool)
            .await?;

        assert_eq!(
            statement, "7s",
            "a job would run without a statement budget"
        );
        assert_eq!(lock, "250ms", "a job would queue behind a lock forever");
        assert_eq!(
            idle, "750ms",
            "a transaction left open would hold its row locks until someone noticed"
        );
        Ok(())
    }

    #[tokio::test]
    #[ignore = "requires a live PostgreSQL"]
    async fn a_query_that_outstays_its_budget_is_cancelled_by_postgres() -> anyhow::Result<()> {
        let (database, pool) = bounded(Duration::from_millis(200));
        let pool = connect(&database, &pool, "jobs-limits-test").await?;

        let started = std::time::Instant::now();
        let outcome = sqlx::query("SELECT pg_sleep(5)").execute(&pool).await;
        let waited = started.elapsed();

        let error =
            outcome.expect_err("a five second sleep cannot fit in two hundred milliseconds");
        assert_eq!(
            error
                .as_database_error()
                .and_then(|error| error.code())
                .as_deref(),
            Some(CANCELLED),
            "the query ended for some other reason than the budget: {error}"
        );
        assert!(
            waited < Duration::from_secs(2),
            "PostgreSQL took {waited:?} to cancel a query budgeted at two hundred milliseconds"
        );
        Ok(())
    }

    #[tokio::test]
    #[ignore = "requires a live PostgreSQL"]
    async fn a_transaction_left_open_is_closed_from_the_other_side() -> anyhow::Result<()> {
        let (database, pool) = bounded(Duration::from_secs(7));
        let pool = connect(&database, &pool, "jobs-limits-test").await?;

        let mut transaction = pool.begin().await?;
        sqlx::query("SELECT 1").execute(&mut *transaction).await?;
        tokio::time::sleep(Duration::from_millis(1_500)).await;
        let outcome = sqlx::query("SELECT 1").execute(&mut *transaction).await;

        assert!(
            outcome.is_err(),
            "a transaction idle for twice its budget was still alive, so a stuck job would \
             hold its locks for as long as it liked"
        );
        Ok(())
    }
}
