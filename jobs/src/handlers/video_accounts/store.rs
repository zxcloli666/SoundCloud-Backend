use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::Serialize;
use sha2::{Digest, Sha256};
use sqlx::PgPool;

use super::Seal;

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

pub(super) async fn list(pool: &PgPool, seal: &Seal) -> anyhow::Result<Vec<AccountRow>> {
    let rows = sqlx::query_file!("queries/video_accounts/list.sql")
        .fetch_all(pool)
        .await?;
    rows.into_iter()
        .map(|row| {
            let secret = seal.open(&row.name, &row.secret)?;
            Ok(AccountRow {
                secret_sha256: hex_prefix(&secret),
                name: row.name,
                source: row.source,
                status: row.status,
                last_checked_at: row.last_checked_at,
                last_error: row.last_error,
            })
        })
        .collect()
}

fn hex_prefix(secret: &str) -> String {
    let digest = Sha256::digest(secret.as_bytes());
    digest
        .iter()
        .take(6)
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

pub(super) async fn secret_of(
    pool: &PgPool,
    seal: &Seal,
    name: &str,
) -> anyhow::Result<Option<(String, String)>> {
    let row = sqlx::query_file!("queries/video_accounts/get.sql", name)
        .fetch_optional(pool)
        .await?;
    row.map(|row| Ok((row.source, seal.open(name, &row.secret)?)))
        .transpose()
}

pub(super) async fn upsert(
    pool: &PgPool,
    seal: &Seal,
    source: &str,
    name: &str,
    secret: &str,
) -> anyhow::Result<bool> {
    let sealed = seal.seal(name, secret)?;
    let row = sqlx::query_file!("queries/video_accounts/upsert.sql", source, name, sealed)
        .fetch_optional(pool)
        .await?;
    Ok(row.is_some())
}

pub(crate) async fn seal_legacy(pool: &PgPool, seal: &Seal) -> anyhow::Result<u64> {
    let rows = sqlx::query_file!("queries/video_accounts/unsealed.sql")
        .fetch_all(pool)
        .await?;
    let count = rows.len() as u64;
    for row in rows {
        let sealed = seal.seal(&row.name, &row.secret)?;
        sqlx::query_file!("queries/video_accounts/reseal.sql", row.name, sealed)
            .execute(pool)
            .await?;
    }
    Ok(count)
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
    seal: &Seal,
    source: &str,
) -> anyhow::Result<Option<(String, String)>> {
    let row = sqlx::query_file!("queries/video_accounts/lease.sql", source)
        .fetch_optional(pool)
        .await?;
    row.map(|row| {
        let secret = seal.open(&row.name, &row.secret)?;
        Ok((row.name, secret))
    })
    .transpose()
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
