use std::collections::BTreeMap;
use std::sync::atomic::{AtomicI64, Ordering};
use std::time::Duration;

use chrono::Utc;

use crate::modules::chat::stores;
use crate::modules::chat::sweeps::media::delete::STUCK_ATTEMPTS;
use crate::modules::chat::{ChatResult, ChatState};

pub const BYTES_EVERY: Duration = Duration::from_secs(300);

static BYTES_AT: AtomicI64 = AtomicI64::new(i64::MIN);

pub async fn refresh(chat: &ChatState) -> ChatResult<()> {
    let stuck = stuck(chat).await?;
    metrics::gauge!("chat_media_delete_stuck").set(stuck as f64);
    for store in &chat.config.media.stores {
        metrics::gauge!("chat_media_cap_bytes", "store" => store.id.to_string())
            .set(stores::cap_bytes(store) as f64);
    }
    let now = Utc::now().timestamp();
    let last = BYTES_AT.load(Ordering::Relaxed);
    if now.saturating_sub(last) < BYTES_EVERY.as_secs() as i64 {
        return Ok(());
    }
    BYTES_AT.store(now, Ordering::Relaxed);
    for (store, bytes) in store_bytes(chat).await? {
        metrics::gauge!("chat_media_bytes", "store" => store.to_string()).set(bytes as f64);
    }
    Ok(())
}

pub async fn stuck(chat: &ChatState) -> ChatResult<i64> {
    Ok(
        sqlx::query_file_scalar!("queries/chat/sweeps/media_stuck.sql", STUCK_ATTEMPTS)
            .fetch_one(&chat.pg)
            .await?,
    )
}

pub async fn store_bytes(chat: &ChatState) -> ChatResult<BTreeMap<i16, i64>> {
    let mut bytes: BTreeMap<i16, i64> = chat
        .config
        .media
        .stores
        .iter()
        .filter_map(|store| i16::try_from(store.id).ok())
        .map(|id| (id, 0))
        .collect();
    for row in sqlx::query_file!("queries/chat/sweeps/media_store_bytes.sql")
        .fetch_all(&chat.pg)
        .await?
    {
        bytes.insert(row.store, row.bytes);
    }
    Ok(bytes)
}
