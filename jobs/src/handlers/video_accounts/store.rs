use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::PgPool;

#[derive(Serialize, sqlx::FromRow)]
pub(super) struct AccountRow {
    pub name: String,
    pub source: String,
    pub status: String,
    pub last_checked_at: Option<DateTime<Utc>>,
    pub last_error: Option<String>,
    pub secret_sha256: String,
}

pub(super) struct Dead {
    pub name: String,
    pub source: String,
    pub last_error: Option<String>,
}

pub(super) async fn list(pool: &PgPool) -> Result<Vec<AccountRow>, sqlx::Error> {
    sqlx::query_file_as!(AccountRow, "queries/video_accounts/list.sql")
        .fetch_all(pool)
        .await
}

pub(super) async fn secret_of(
    pool: &PgPool,
    name: &str,
) -> Result<Option<(String, String)>, sqlx::Error> {
    let row = sqlx::query_file!("queries/video_accounts/get.sql", name)
        .fetch_optional(pool)
        .await?;
    Ok(row.map(|row| (row.source, row.secret)))
}

pub(super) async fn upsert(
    pool: &PgPool,
    source: &str,
    name: &str,
    secret: &str,
) -> Result<bool, sqlx::Error> {
    let row = sqlx::query_file!("queries/video_accounts/upsert.sql", source, name, secret)
        .fetch_optional(pool)
        .await?;
    Ok(row.is_some())
}

pub(super) async fn due(pool: &PgPool, every: Duration) -> Result<Vec<String>, sqlx::Error> {
    sqlx::query_file_scalar!("queries/video_accounts/due.sql", every.as_secs_f64())
        .fetch_all(pool)
        .await
}

pub(crate) async fn record(
    pool: &PgPool,
    name: &str,
    status: Option<&str>,
    error: Option<&str>,
) -> Result<(), sqlx::Error> {
    sqlx::query_file!("queries/video_accounts/record.sql", name, status, error)
        .execute(pool)
        .await?;
    Ok(())
}

pub(super) async fn claim_alerts(pool: &PgPool) -> Result<Vec<Dead>, sqlx::Error> {
    let rows = sqlx::query_file!("queries/video_accounts/claim_alerts.sql")
        .fetch_all(pool)
        .await?;
    Ok(rows
        .into_iter()
        .map(|row| Dead {
            name: row.name,
            source: row.source,
            last_error: row.last_error,
        })
        .collect())
}

pub(super) async fn release_alert(pool: &PgPool, name: &str) -> Result<(), sqlx::Error> {
    sqlx::query_file!("queries/video_accounts/release_alert.sql", name)
        .execute(pool)
        .await?;
    Ok(())
}

pub(crate) async fn lease(
    pool: &PgPool,
    source: &str,
) -> Result<Option<(String, String)>, sqlx::Error> {
    let row = sqlx::query_file!("queries/video_accounts/lease.sql", source)
        .fetch_optional(pool)
        .await?;
    Ok(row.map(|row| (row.name, row.secret)))
}

pub(crate) async fn pause(
    pool: &PgPool,
    name: &str,
    pause: Duration,
    error: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query_file!(
        "queries/video_accounts/pause.sql",
        name,
        pause.as_secs_f64(),
        error
    )
    .execute(pool)
    .await?;
    Ok(())
}

pub(crate) async fn kill(pool: &PgPool, name: &str, error: &str) -> Result<(), sqlx::Error> {
    record(pool, name, Some("dead"), Some(error)).await
}
