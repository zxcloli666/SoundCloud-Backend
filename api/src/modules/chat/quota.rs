use sqlx::{PgConnection, PgPool};
use uuid::Uuid;

use crate::modules::chat::media::ledger;
use crate::modules::chat::media::limits::{
    ATTACH_WINDOW, DAY_BYTES_MAX, QUOTA_BYTES, RESTORE_WINDOW,
};
use crate::modules::chat::media::wire::Picked;
use crate::modules::chat::{ChatError, ChatResult, ChatState, stores};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Used {
    pub bytes: u64,
    pub reserved: u64,
}

impl Used {
    pub fn total(self) -> u64 {
        self.bytes + self.reserved
    }

    fn of(bytes: i64, reserved: i64) -> Self {
        Self {
            bytes: unsigned(bytes),
            reserved: unsigned(reserved),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Released {
    pub roots: u32,
    pub freed: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Restored {
    pub roots: u32,
    pub used: Used,
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

pub fn quota() -> u64 {
    QUOTA_BYTES
}

pub async fn write_store_open(chat: &ChatState) -> ChatResult<bool> {
    Ok(stores::open_for_writes(chat).await?.is_some())
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

pub async fn commit(pg: &PgPool, user_id: &str, root: Uuid) -> ChatResult<Option<Used>> {
    let mut tx = pg.begin().await?;
    ledger::lock(&mut tx, &[root]).await?;
    let row = sqlx::query_file!(
        "queries/chat/media/commit_flip.sql",
        root,
        user_id,
        seconds(ATTACH_WINDOW)
    )
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    if row.files == 0 {
        return Ok(None);
    }
    match (row.bytes, row.reserved) {
        (Some(bytes), Some(reserved)) => Ok(Some(Used::of(bytes, reserved))),
        _ => Ok(Some(used_in(pg, user_id).await?)),
    }
}

pub async fn release(pg: &PgPool, user_id: &str, picked: &Picked) -> ChatResult<(Released, Used)> {
    let (ids, all) = picked.parts();
    let mut tx = pg.begin().await?;
    let roots = sqlx::query_file_scalar!("queries/chat/media/delete_lock.sql", user_id, ids, all)
        .fetch_all(&mut *tx)
        .await?;
    if roots.is_empty() {
        tx.rollback().await?;
        let nothing = Released { roots: 0, freed: 0 };
        return Ok((nothing, used_in(pg, user_id).await?));
    }
    let row = sqlx::query_file!(
        "queries/chat/media/delete_rows.sql",
        user_id,
        &roots,
        seconds(RESTORE_WINDOW)
    )
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    let released = Released {
        roots: u32::try_from(row.deleted).unwrap_or(0),
        freed: unsigned(row.freed),
    };
    let used = match (row.bytes, row.reserved) {
        (Some(bytes), Some(reserved)) => Used::of(bytes, reserved),
        _ => used_in(pg, user_id).await?,
    };
    Ok((released, used))
}

pub async fn restore(pg: &PgPool, user_id: &str, picked: &Picked) -> ChatResult<Restored> {
    let (ids, all) = picked.parts();
    let mut tx = pg.begin().await?;
    let roots = sqlx::query_file_scalar!("queries/chat/media/restore_lock.sql", user_id, ids, all)
        .fetch_all(&mut *tx)
        .await?;
    if roots.is_empty() {
        tx.rollback().await?;
        return Ok(Restored {
            roots: 0,
            used: used_in(pg, user_id).await?,
        });
    }
    let row = sqlx::query_file!(
        "queries/chat/media/restore_rows.sql",
        user_id,
        &roots,
        signed(QUOTA_BYTES),
        seconds(ATTACH_WINDOW)
    )
    .fetch_one(&mut *tx)
    .await?;
    let (Some(bytes), Some(reserved)) = (row.bytes, row.reserved) else {
        tx.rollback().await?;
        return Err(ChatError::QuotaExceeded {
            used: used_in(pg, user_id).await?.total(),
            quota: QUOTA_BYTES,
        });
    };
    tx.commit().await?;
    Ok(Restored {
        roots: u32::try_from(row.restored).unwrap_or(0),
        used: Used::of(bytes, reserved),
    })
}

#[cfg(test)]
#[path = "tests/media_quota_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/media_race_tests.rs"]
mod race_tests;
