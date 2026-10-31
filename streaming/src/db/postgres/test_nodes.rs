use std::fs;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::atomic::{AtomicUsize, Ordering};

use deadpool_postgres::{Config as PgConfig, Runtime};
use tokio::sync::OnceCell;
use tokio_postgres::types::FromSql;
use tokio_postgres::{Client, NoTls};

use super::{PgPool, pool_config};
use crate::source_tree::crate_root;

const TEMPLATE_LOCK: i64 = 7_424_208;

static TEMPLATE: OnceCell<String> = OnceCell::const_new();
static CASES: AtomicUsize = AtomicUsize::new(0);

pub struct Node {
    pub pg: PgPool,
    pub sql: Client,
    name: String,
}

pub async fn core_and_api(fixture: &str) -> (Node, Node) {
    let template = TEMPLATE.get_or_init(build_template).await;
    let case = format!(
        "stream_{}_{}",
        std::process::id(),
        CASES.fetch_add(1, Ordering::Relaxed)
    );
    let core = format!("{case}_core");
    let api = format!("{case}_api");
    let admin = connect(&server_url()).await;
    run(
        &admin,
        &format!("CREATE DATABASE {core} TEMPLATE {template}"),
    )
    .await;
    let seed = connect(&url_of(&core)).await;
    run(
        &seed,
        "SELECT count(*) FROM public.scd_configure('core', 'core', 'split')",
    )
    .await;
    run(&seed, fixture).await;
    drop(seed);
    run(&admin, &format!("CREATE DATABASE {api} TEMPLATE {core}")).await;
    let configure = connect(&url_of(&api)).await;
    run(
        &configure,
        "SELECT count(*) FROM public.scd_configure('main', 'api', 'split')",
    )
    .await;
    drop(configure);
    (Node::open(core).await, Node::open(api).await)
}

impl Node {
    async fn open(name: String) -> Node {
        let url = url_of(&name);
        let mut config = PgConfig::new();
        config.url = Some(url.clone());
        config.pool = Some(pool_config(4, 10));
        let pool = config
            .create_pool(Some(Runtime::Tokio1), NoTls)
            .expect("a pool for the test database");
        Node {
            pg: PgPool { pool },
            sql: connect(&url).await,
            name,
        }
    }

    pub async fn value<T: for<'a> FromSql<'a>>(&self, query: &str) -> T {
        self.sql
            .query_one(query, &[])
            .await
            .unwrap_or_else(|e| panic!("{query}: {e}"))
            .get(0)
    }

    pub async fn finish(self) {
        let Node { pg, sql, name } = self;
        drop(pg);
        drop(sql);
        let admin = connect(&server_url()).await;
        run(
            &admin,
            &format!("DROP DATABASE IF EXISTS {name} WITH (FORCE)"),
        )
        .await;
    }
}

fn server_url() -> String {
    std::env::var("STREAMING_SCHEMA_TEST_DATABASE_URL")
        .expect("STREAMING_SCHEMA_TEST_DATABASE_URL names a Postgres 17 superuser")
}

fn url_of(database: &str) -> String {
    let mut url = url::Url::parse(&server_url()).expect("a postgres:// url");
    url.set_path(database);
    url.to_string()
}

async fn connect(url: &str) -> Client {
    let (client, connection) = tokio_postgres::connect(url, NoTls)
        .await
        .unwrap_or_else(|e| panic!("connect {url}: {e}"));
    tokio::spawn(connection);
    client
}

async fn run(client: &Client, query: &str) {
    client
        .batch_execute(query)
        .await
        .unwrap_or_else(|e| panic!("{query}: {e}"));
}

async fn build_template() -> String {
    let migrations = migrations();
    let mut hasher = DefaultHasher::new();
    migrations.hash(&mut hasher);
    let name = format!("stream_tpl_{:016x}", hasher.finish());
    let admin = connect(&server_url()).await;
    run(&admin, &format!("SELECT pg_advisory_lock({TEMPLATE_LOCK})")).await;
    let ready: bool = admin
        .query_one(
            "SELECT EXISTS (SELECT 1 FROM pg_database WHERE datname = $1)",
            &[&name],
        )
        .await
        .expect("the template lookup")
        .get(0);
    if !ready {
        let building = format!("{name}_build");
        run(
            &admin,
            &format!("DROP DATABASE IF EXISTS {building} WITH (FORCE)"),
        )
        .await;
        run(&admin, &format!("CREATE DATABASE {building}")).await;
        let db = connect(&url_of(&building)).await;
        run(&db, "SET client_min_messages = error").await;
        for (file, sql) in &migrations {
            db.batch_execute(sql)
                .await
                .unwrap_or_else(|e| panic!("{file}: {e}"));
        }
        drop(db);
        run(
            &admin,
            &format!("ALTER DATABASE {building} RENAME TO {name}"),
        )
        .await;
    }
    run(
        &admin,
        &format!("SELECT pg_advisory_unlock({TEMPLATE_LOCK})"),
    )
    .await;
    name
}

fn migrations() -> Vec<(String, String)> {
    let dir = crate_root().join("../api/migrations");
    let mut files: Vec<(String, String)> = fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
        .map(|entry| entry.expect("a migration entry").path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "sql"))
        .map(|path| {
            let name = path
                .file_name()
                .expect("a migration file name")
                .to_string_lossy()
                .into_owned();
            let body = fs::read_to_string(&path).expect("a readable migration");
            (name, body)
        })
        .collect();
    files.sort();
    files
}
