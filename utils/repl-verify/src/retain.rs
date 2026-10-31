use std::convert::Infallible;
use std::time::Duration;

use sqlx::PgPool;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

pub const HELD_REASONS: [&str; 7] = [
    "recovery",
    "role",
    "subscription",
    "slot",
    "resend",
    "pending",
    "other",
];

const DELETED: &str = "scd_repl_retention_deleted_total";
const REDACTED: &str = "scd_repl_retention_redacted_total";
const HELD: &str = "scd_repl_retention_held";
const TABLES: [&str; 2] = ["cmd_outbox", "oauth_app_tokens_local"];
const BATCH: i32 = 1_000;
const ROUNDS: u32 = 200;
const PAUSE: Duration = Duration::from_millis(500);
const FIRST_RUN: Duration = Duration::from_secs(60);
const PACE: Duration = Duration::from_secs(600);

#[derive(Debug)]
pub struct Retention {
    pub deleted: i64,
    pub redacted: i64,
    pub held: Option<String>,
    pub reason: Option<String>,
}

impl Retention {
    fn full(&self, batch: i32) -> bool {
        self.deleted + self.redacted >= i64::from(batch)
    }
}

pub async fn retain_once(pool: &PgPool, table: &str, batch: i32) -> Result<Retention, sqlx::Error> {
    sqlx::query_file_as!(Retention, "queries/node_retention.sql", table, batch)
        .fetch_one(pool)
        .await
}

pub fn register_retention(table: &'static str) {
    metrics::counter!(DELETED, "table" => table).increment(0);
    metrics::counter!(REDACTED, "table" => table).increment(0);
    for reason in HELD_REASONS {
        metrics::gauge!(HELD, "table" => table, "reason" => reason).set(0.0);
    }
}

pub fn record_retention(table: &'static str, deleted: u64, redacted: u64, reason: Option<&str>) {
    metrics::counter!(DELETED, "table" => table).increment(deleted);
    metrics::counter!(REDACTED, "table" => table).increment(redacted);
    for known in HELD_REASONS {
        let held = f64::from(u8::from(reason == Some(known)));
        metrics::gauge!(HELD, "table" => table, "reason" => known).set(held);
    }
}

pub struct Retainer {
    pool: PgPool,
    pace: Duration,
}

impl Retainer {
    pub fn new(pool: PgPool) -> Self {
        Self { pool, pace: PACE }
    }

    pub async fn run(self, stop: CancellationToken) -> Result<(), Infallible> {
        for table in TABLES {
            register_retention(table);
        }
        let mut wait = FIRST_RUN;
        loop {
            tokio::select! {
                () = stop.cancelled() => return Ok(()),
                () = tokio::time::sleep(wait) => {}
            }
            for table in TABLES {
                if let Err(error) = self.retain(table, &stop).await {
                    warn!(%error, table, "scd node retention failed");
                }
            }
            wait = self.pace;
        }
    }

    async fn retain(
        &self,
        table: &'static str,
        stop: &CancellationToken,
    ) -> Result<(), sqlx::Error> {
        let mut deleted = 0;
        let mut redacted = 0;
        let mut rounds = 0;
        let last = loop {
            let batch = retain_once(&self.pool, table, BATCH).await?;
            deleted += batch.deleted;
            redacted += batch.redacted;
            rounds += 1;
            if !batch.full(BATCH) || rounds == ROUNDS || stop.is_cancelled() {
                break batch;
            }
            tokio::time::sleep(PAUSE).await;
        };
        report(table, deleted, redacted, &last);
        Ok(())
    }
}

fn report(table: &'static str, deleted: i64, redacted: i64, last: &Retention) {
    let deleted = u64::try_from(deleted).unwrap_or(0);
    let redacted = u64::try_from(redacted).unwrap_or(0);
    record_retention(table, deleted, redacted, last.reason.as_deref());
    match &last.held {
        Some(held) => warn!(table, %held, deleted, redacted, "scd node retention held back"),
        None if deleted + redacted > 0 => info!(table, deleted, redacted, "scd node retention"),
        None => {}
    }
}
