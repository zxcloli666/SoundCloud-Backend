use std::sync::{Arc, LazyLock};
use std::time::Duration;

use redis::Script;
use uuid::Uuid;

use crate::modules::chat::{ChatResult, ChatState, rt_keys};

pub const RENEW_EVERY: Duration = Duration::from_secs(10);

const HOLD_SCRIPT: &str = r"
if redis.call('GET', KEYS[1]) == ARGV[1] then
  redis.call('PEXPIRE', KEYS[1], ARGV[2])
  return 1
end
if redis.call('SET', KEYS[1], ARGV[1], 'NX', 'PX', ARGV[2]) then
  return 1
end
return 0
";

const RELEASE_SCRIPT: &str = r"
if redis.call('GET', KEYS[1]) == ARGV[1] then
  return redis.call('DEL', KEYS[1])
end
return 0
";

static TOKEN: LazyLock<String> = LazyLock::new(|| Uuid::now_v7().to_string());
static HOLD: LazyLock<Script> = LazyLock::new(|| Script::new(HOLD_SCRIPT));
static RELEASE: LazyLock<Script> = LazyLock::new(|| Script::new(RELEASE_SCRIPT));

pub async fn hold(chat: &ChatState) -> ChatResult<bool> {
    hold_as(chat, &TOKEN).await
}

pub async fn release(chat: &ChatState) -> ChatResult<bool> {
    release_as(chat, &TOKEN).await
}

pub async fn hold_as(chat: &ChatState, token: &str) -> ChatResult<bool> {
    let lease_ms = u64::try_from(rt_keys::LEAD_TTL.as_millis()).unwrap_or(u64::MAX);
    let mut conn = chat.redis.get().await?;
    let held: i64 = HOLD
        .key(rt_keys::LEAD)
        .arg(token)
        .arg(lease_ms)
        .invoke_async(&mut conn)
        .await?;
    Ok(held == 1)
}

pub async fn release_as(chat: &ChatState, token: &str) -> ChatResult<bool> {
    let mut conn = chat.redis.get().await?;
    let released: i64 = RELEASE
        .key(rt_keys::LEAD)
        .arg(token)
        .invoke_async(&mut conn)
        .await?;
    Ok(released == 1)
}

pub async fn keep(chat: Arc<ChatState>) {
    loop {
        if let Err(error) = hold(&chat).await {
            tracing::warn!(code = error.code(), "chat sweep lease was not renewed");
        }
        tokio::select! {
            _ = chat.shutdown.cancelled() => break,
            _ = tokio::time::sleep(RENEW_EVERY) => {}
        }
    }
    if let Err(error) = release(&chat).await {
        tracing::warn!(code = error.code(), "chat sweep lease was not released");
    }
}
