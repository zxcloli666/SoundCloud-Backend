use sqlx::{PgConnection, PgPool};

use crate::modules::chat::media::limits::{DAY_BYTES_MAX, QUOTA_BYTES};
use crate::modules::chat::{ChatError, ChatResult, ChatState};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Used {
    pub bytes: u64,
    pub reserved: u64,
}

impl Used {
    pub fn total(self) -> u64 {
        self.bytes + self.reserved
    }

    pub fn of(bytes: i64, reserved: i64) -> Self {
        Self {
            bytes: unsigned(bytes),
            reserved: unsigned(reserved),
        }
    }
}

pub fn unsigned(value: i64) -> u64 {
    u64::try_from(value).unwrap_or(0)
}

pub fn signed(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

pub fn seconds(window: std::time::Duration) -> i32 {
    i32::try_from(window.as_secs()).unwrap_or(i32::MAX)
}

pub async fn used(chat: &ChatState, user_id: &str) -> ChatResult<Used> {
    used_in(&chat.pg, user_id).await
}

pub async fn used_in(pg: &PgPool, user_id: &str) -> ChatResult<Used> {
    let row = sqlx::query_file!("queries/chat/media/usage_of.sql", user_id)
        .fetch_optional(pg)
        .await?;
    Ok(row.map_or(Used::of(0, 0), |row| Used::of(row.bytes, row.reserved)))
}

pub async fn reserve(tx: &mut PgConnection, user_id: &str, total: u64) -> ChatResult<Used> {
    let granted = sqlx::query_file!(
        "queries/chat/media/reserve_quota.sql",
        user_id,
        signed(total),
        signed(QUOTA_BYTES),
        signed(DAY_BYTES_MAX)
    )
    .fetch_optional(&mut *tx)
    .await?;
    if let Some(row) = granted {
        return Ok(Used::of(row.bytes, row.reserved));
    }
    let row = sqlx::query_file!("queries/chat/media/reserve_refused.sql", user_id)
        .fetch_optional(&mut *tx)
        .await?;
    let (before, day_bytes) = row.map_or((Used::of(0, 0), 0), |row| {
        (Used::of(row.bytes, row.reserved), unsigned(row.day_bytes))
    });
    if day_bytes + total > DAY_BYTES_MAX {
        metrics::counter!("chat_limit_rejects_total", "bucket" => "media_day_bytes").increment(1);
        return Err(ChatError::SendLimited);
    }
    Err(ChatError::QuotaExceeded {
        used: before.total(),
        quota: QUOTA_BYTES,
    })
}

#[cfg(test)]
#[path = "tests/media_quota_tests.rs"]
mod tests;
