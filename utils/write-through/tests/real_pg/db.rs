use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::Path;

use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{Connection, PgConnection, PgExecutor, PgPool};
use tokio::sync::OnceCell;

const TEST_SQL: &str = include_str!("kinds.sql");
const TEMPLATE_LOCK: i64 = 7424201;

static TEMPLATE: OnceCell<String> = OnceCell::const_new();

fn server_url() -> String {
    env::var("DATABASE_URL")
        .expect("DATABASE_URL names a Postgres 17 superuser with track_commit_timestamp=on")
}

pub fn options(db: &str) -> PgConnectOptions {
    server_url()
        .parse::<PgConnectOptions>()
        .expect("DATABASE_URL")
        .database(db)
}

pub async fn connect(db: &str) -> PgConnection {
    PgConnection::connect_with(&options(db))
        .await
        .unwrap_or_else(|e| panic!("connect {db}: {e}"))
}

pub async fn pool(db: &str) -> PgPool {
    PgPoolOptions::new()
        .max_connections(4)
        .connect_with(options(db))
        .await
        .unwrap_or_else(|e| panic!("pool {db}: {e}"))
}

pub async fn admin() -> PgConnection {
    PgConnection::connect_with(
        &server_url()
            .parse::<PgConnectOptions>()
            .expect("DATABASE_URL"),
    )
    .await
    .expect("connect to the server")
}

pub async fn tokio_client(db: &str) -> tokio_postgres::Client {
    let mut config = server_url()
        .parse::<tokio_postgres::Config>()
        .expect("DATABASE_URL");
    config.dbname(db);
    let (client, connection) = config
        .connect(tokio_postgres::NoTls)
        .await
        .expect("tokio-postgres connect");
    tokio::spawn(connection);
    client
}

pub async fn execute(conn: &mut PgConnection, sql: &str) {
    sqlx::raw_sql(sql)
        .execute(conn)
        .await
        .unwrap_or_else(|e| panic!("{sql}: {e}"));
}

pub async fn hashes(db: impl PgExecutor<'_>) -> BTreeMap<String, String> {
    sqlx::query_as::<_, (String, String)>("SELECT tbl, hash FROM wt_test.table_hashes()")
        .fetch_all(db)
        .await
        .expect("table hashes")
        .into_iter()
        .collect()
}

pub async fn template() -> &'static str {
    TEMPLATE.get_or_init(build_template).await
}

async fn build_template() -> String {
    let migrations = migrations();
    let mut hasher = DefaultHasher::new();
    migrations.hash(&mut hasher);
    TEST_SQL.hash(&mut hasher);
    let name = format!("wt_tpl_{:016x}", hasher.finish());
    let mut admin = admin().await;
    sqlx::query("SELECT pg_advisory_lock($1)")
        .bind(TEMPLATE_LOCK)
        .execute(&mut admin)
        .await
        .unwrap();
    let ready: bool =
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM pg_database WHERE datname = $1)")
            .bind(&name)
            .fetch_one(&mut admin)
            .await
            .unwrap();
    if !ready {
        let building = format!("{name}_build");
        execute(
            &mut admin,
            &format!("DROP DATABASE IF EXISTS {building} WITH (FORCE)"),
        )
        .await;
        execute(&mut admin, &format!("CREATE DATABASE {building}")).await;
        let mut db = connect(&building).await;
        execute(&mut db, "SET client_min_messages = error").await;
        for (file, sql) in &migrations {
            sqlx::raw_sql(sql)
                .execute(&mut db)
                .await
                .unwrap_or_else(|e| panic!("{file}: {e}"));
        }
        execute(&mut db, TEST_SQL).await;
        db.close().await.unwrap();
        execute(
            &mut admin,
            &format!("ALTER DATABASE {building} RENAME TO {name}"),
        )
        .await;
    }
    sqlx::query("SELECT pg_advisory_unlock($1)")
        .bind(TEMPLATE_LOCK)
        .execute(&mut admin)
        .await
        .unwrap();
    admin.close().await.unwrap();
    name
}

fn migrations() -> Vec<(String, String)> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../api/migrations");
    let mut files: Vec<(String, String)> = fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "sql"))
        .map(|path| {
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            (name, fs::read_to_string(&path).unwrap())
        })
        .collect();
    files.sort();
    files
}
