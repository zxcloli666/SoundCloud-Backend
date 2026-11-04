use crate::modules::chat::rt_keys::{REPLAY_TTL, replay};
use crate::modules::chat::{ChatError, ChatResult, ChatState};

pub async fn claim(chat: &ChatState, enc: &[u8; 32]) -> ChatResult<()> {
    let mut redis = chat.redis.get().await?;
    let fresh: Option<String> = redis::cmd("SET")
        .arg(replay(enc))
        .arg(1)
        .arg("NX")
        .arg("EX")
        .arg(REPLAY_TTL.as_secs())
        .query_async(&mut *redis)
        .await?;
    match fresh {
        Some(_) => Ok(()),
        None => {
            metrics::counter!("chat_replay_total").increment(1);
            Err(ChatError::Replay)
        }
    }
}
