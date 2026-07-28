//! Standalone migration runner — discrete deploy step.
//!
//! Run this BEFORE starting the app (with `MIGRATE_ON_BOOT=false` set on the app),
//! so a failing migration fails the deploy step without taking down the currently
//! serving instance. Reuses the same advisory locks as the in-app runner, so a boot
//! migrate and this bin never race.
//!
//! Катит core-БД (`DATABASE_URL`) и, если задана, ops-БД (`OPS_DATABASE_URL`).
//! Ops не задана — просто пропускаем, это штатный режим main/star.

use std::time::Duration;

use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;

/// Must stay in sync with `db::advisory_locks`.
const MIGRATIONS_LOCK: i64 = 0x5343_445F_4D49;
const MIGRATIONS_OPS_LOCK: i64 = 0x5343_445F_4D4F;

/// Оба мигратора — `ignore_missing`, чтобы core и ops могли делить одну базу.
/// Обоснование см. в `db::CORE_MIGRATOR`.
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

#[tokio::main]
async fn main() {
    let url = match std::env::var("DATABASE_URL") {
        Ok(u) if !u.is_empty() => u,
        _ => {
            eprintln!("migrate: DATABASE_URL must be set");
            std::process::exit(1);
        }
    };

    run("core", &url, MIGRATIONS_LOCK, &CORE_MIGRATOR).await;

    match std::env::var("OPS_DATABASE_URL") {
        Ok(ops_url) if !ops_url.is_empty() => {
            run("ops", &ops_url, MIGRATIONS_OPS_LOCK, &OPS_MIGRATOR).await;
        }
        _ => println!("migrate: OPS_DATABASE_URL not set — ops migrations skipped"),
    }
}

async fn run(name: &str, url: &str, lock: i64, migrator: &sqlx::migrate::Migrator) {
    let pool = connect(name, url).await;

    let mut conn = pool.acquire().await.unwrap_or_else(|e| {
        eprintln!("migrate[{name}]: acquire failed: {e}");
        std::process::exit(1);
    });

    if let Err(e) = sqlx::query("SELECT pg_advisory_lock($1)")
        .bind(lock)
        .execute(&mut *conn)
        .await
    {
        eprintln!("migrate[{name}]: advisory lock failed: {e}");
        std::process::exit(1);
    }

    let result = migrator.run(&mut *conn).await;

    // Best-effort unlock; the session ending releases it anyway.
    let _ = sqlx::query("SELECT pg_advisory_unlock($1)")
        .bind(lock)
        .execute(&mut *conn)
        .await;

    match result {
        Ok(()) => println!("migrate[{name}]: migrations applied"),
        Err(e) => {
            eprintln!("migrate[{name}]: failed: {e}");
            std::process::exit(1);
        }
    }
}

async fn connect(name: &str, url: &str) -> PgPool {
    PgPoolOptions::new()
        .max_connections(1)
        .acquire_timeout(Duration::from_secs(30))
        .connect(url)
        .await
        .unwrap_or_else(|e| {
            eprintln!("migrate[{name}]: connect failed: {e}");
            std::process::exit(1);
        })
}
