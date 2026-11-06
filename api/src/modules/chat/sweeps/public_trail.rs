use std::sync::Arc;
use std::time::Duration;

use crate::modules::chat::ChatState;
use crate::modules::chat::public_chat::delivery;

pub const TICK: Duration = Duration::from_millis(250);

const FAILED_TOTAL: &str = "chat_public_trail_failed_total";

pub async fn run(chat: Arc<ChatState>) {
    if !chat.config.public {
        return;
    }
    loop {
        tokio::select! {
            () = chat.shutdown.cancelled() => return,
            () = tokio::time::sleep(TICK) => {}
        }
        if let Err(error) = once(&chat).await {
            metrics::counter!(FAILED_TOTAL).increment(1);
            tracing::warn!(code = error.code(), "a public chat wake was not sent");
        }
    }
}

pub async fn once(chat: &ChatState) -> crate::modules::chat::ChatResult<bool> {
    if !delivery::trail_due(chat).await? {
        return Ok(false);
    }
    delivery::flush(chat).await?;
    Ok(true)
}
