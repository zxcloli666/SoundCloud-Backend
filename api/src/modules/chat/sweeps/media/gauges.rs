use crate::modules::chat::stores;
use crate::modules::chat::sweeps::media::delete::STUCK_ATTEMPTS;
use crate::modules::chat::{ChatResult, ChatState};

pub async fn refresh(chat: &ChatState) -> ChatResult<()> {
    let stuck = stuck(chat).await?;
    metrics::gauge!("chat_media_delete_stuck").set(stuck as f64);
    for store in &chat.config.media.stores {
        let id = store.id.to_string();
        metrics::gauge!("chat_media_cap_bytes", "store" => id.clone())
            .set(stores::cap_bytes(store) as f64);
        metrics::gauge!("chat_media_fill_bytes", "store" => id)
            .set(stores::filled(chat, store.id).await? as f64);
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
