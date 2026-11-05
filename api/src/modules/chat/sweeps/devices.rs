use std::collections::BTreeMap;
use std::time::Duration;

use chrono::{DateTime, Utc};
use tokio::time::Instant;
use uuid::Uuid;

use crate::modules::chat::sweeps::hooks;
use crate::modules::chat::{ChatResult, ChatState};

pub const EVERY_SECS: u64 = 3_600;
pub const BATCH_ROWS: i64 = 1_000;
pub const RUN_TIME: Duration = Duration::from_secs(300);

pub async fn run_once(chat: &ChatState) -> ChatResult<u64> {
    let pass = Utc::now();
    let deadline = Instant::now() + RUN_TIME;
    let mut revoked = 0;
    while Instant::now() < deadline && !chat.shutdown.is_cancelled() {
        let (checked, batch) = check_batch(chat, pass).await?;
        revoked += batch.values().map(Vec::len).sum::<usize>() as u64;
        for (user_id, ids) in &batch {
            hooks::devices_revoked(chat, user_id, ids).await?;
        }
        if checked < BATCH_ROWS as usize {
            break;
        }
    }
    Ok(revoked)
}

async fn check_batch(
    chat: &ChatState,
    pass: DateTime<Utc>,
) -> ChatResult<(usize, BTreeMap<String, Vec<Uuid>>)> {
    let mut tx = chat.pg.begin().await?;
    let stale = sqlx::query_file!("queries/chat/sweeps/stale_devices.sql", pass, BATCH_ROWS)
        .fetch_all(&mut *tx)
        .await?;
    let ids: Vec<Uuid> = stale.iter().map(|row| row.id).collect();
    let sessions: Vec<Vec<u8>> = stale.iter().map(|row| row.session_hash.clone()).collect();
    let rows = sqlx::query_file!(
        "queries/chat/sweeps/revoke_sessionless.sql",
        &ids,
        &sessions
    )
    .fetch_all(&mut *tx)
    .await?;
    sqlx::query_file!("queries/chat/sweeps/mark_alive_checked.sql", &ids, pass)
        .execute(&mut *tx)
        .await?;
    let mut revoked: BTreeMap<String, Vec<Uuid>> = BTreeMap::new();
    for row in rows {
        revoked.entry(row.user_id).or_default().push(row.id);
    }
    if !revoked.is_empty() {
        let users: Vec<String> = revoked.keys().cloned().collect();
        sqlx::query_file!("queries/chat/sweeps/rekey_users.sql", &users)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok((stale.len(), revoked))
}
