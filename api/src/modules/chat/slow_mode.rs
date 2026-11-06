use redis::AsyncCommands;
use uuid::Uuid;

use crate::modules::chat::{ChatError, ChatResult, ChatState, rt_keys};

pub async fn hold(
    chat: &ChatState,
    conversation: Uuid,
    user_id: &str,
    seconds: u32,
) -> ChatResult<()> {
    let key = rt_keys::slow_mode(conversation, user_id);
    let mut conn = chat.redis.get().await?;
    let taken: Option<String> = redis::cmd("SET")
        .arg(&key)
        .arg(1)
        .arg("NX")
        .arg("PX")
        .arg(u64::from(seconds) * 1_000)
        .query_async(&mut conn)
        .await?;
    if taken.is_some() {
        return Ok(());
    }
    let left: i64 = conn.pttl(&key).await?;
    Err(ChatError::SlowMode {
        retry_ms: u64::try_from(left).unwrap_or(0).max(1),
    })
}
