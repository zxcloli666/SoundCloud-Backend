use std::time::Duration;

use chrono::{DateTime, NaiveDate, Utc};
use sqlx::PgPool;
use tracing::{info, warn};

use super::anthropic::Spend;
use crate::handlers::video_accounts::Notify;
use crate::queue::{JobError, JobResult};

pub const BUDGET: &str = "budget";
pub const API_DOWN: &str = "api_down";

#[derive(Debug, PartialEq, Eq)]
pub struct Paused {
    pub reason: String,
    pub due: bool,
}

pub fn period(now: DateTime<Utc>, reset_day: u32) -> NaiveDate {
    llm_ladder::month_start(now, reset_day).date_naive()
}

pub async fn spent(pool: &PgPool, ladder: &str, period: NaiveDate) -> JobResult<f64> {
    let row = sqlx::query_file!("queries/llm/spent.sql", ladder, period)
        .fetch_one(pool)
        .await
        .map_err(JobError::retryable)?;
    Ok(row.usd)
}

pub async fn record(
    pool: &PgPool,
    ladder: &str,
    period: NaiveDate,
    track: &str,
    spend: &[Spend],
) -> JobResult {
    for call in spend {
        sqlx::query_file!(
            "queries/llm/spend_add.sql",
            ladder,
            period,
            track,
            call.call,
            call.provider,
            call.model,
            call.input_tokens,
            call.output_tokens,
            call.cache_read_tokens,
            call.cache_write_tokens,
            call.usd
        )
        .execute(pool)
        .await
        .map_err(JobError::retryable)?;
    }
    Ok(())
}

pub async fn paused(pool: &PgPool, ladder: &str) -> JobResult<Option<Paused>> {
    let row = sqlx::query_file!("queries/llm/pause_get.sql", ladder)
        .fetch_optional(pool)
        .await
        .map_err(JobError::retryable)?;
    Ok(row.map(|row| Paused {
        reason: row.reason,
        due: row.due,
    }))
}

pub async fn pause<N: Notify>(
    pool: &PgPool,
    notify: &N,
    ladder: &str,
    reason: &'static str,
    wait: Duration,
    text: &str,
) -> JobResult {
    let row = sqlx::query_file!(
        "queries/llm/pause_set.sql",
        ladder,
        reason,
        wait.as_secs_f64()
    )
    .fetch_one(pool)
    .await
    .map_err(JobError::retryable)?;
    if row.alerted {
        return Ok(());
    }
    warn!(ladder, reason, "an LLM ladder is paused");
    match notify.send(text).await {
        Ok(()) => {
            sqlx::query_file!("queries/llm/pause_alerted.sql", ladder)
                .execute(pool)
                .await
                .map_err(JobError::retryable)?;
        }
        Err(error) => warn!(ladder, %error, "an LLM ladder pause was not sent to telegram"),
    }
    Ok(())
}

pub async fn resume(pool: &PgPool, ladder: &str) -> JobResult {
    let done = sqlx::query_file!("queries/llm/pause_clear.sql", ladder)
        .execute(pool)
        .await
        .map_err(JobError::retryable)?;
    if done.rows_affected() > 0 {
        info!(ladder, "an LLM ladder resumed");
    }
    Ok(())
}
