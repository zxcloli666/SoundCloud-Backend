use std::future::Future;
use std::sync::LazyLock;
use std::time::Duration;

use redis::Script;
use uuid::Uuid;

use crate::modules::chat::errors::BusySource;
use crate::modules::chat::{ChatError, ChatResult, ChatState, rt_keys};

pub const LEASE: Duration = Duration::from_secs(30 * 60);

const RELEASE_SCRIPT: &str = r"
if redis.call('GET', KEYS[1]) == ARGV[1] then
  return redis.call('DEL', KEYS[1])
end
return 0
";

static RELEASE: LazyLock<Script> = LazyLock::new(|| Script::new(RELEASE_SCRIPT));

pub async fn exclusive<T>(
    chat: &ChatState,
    work: impl Future<Output = ChatResult<T>>,
) -> ChatResult<T> {
    let token = Uuid::now_v7().to_string();
    if !take(chat, &token).await? {
        return Err(ChatError::busy(BusySource::LoadtestLease));
    }
    let result = work.await;
    if let Err(error) = release(chat, &token).await {
        tracing::warn!(code = error.code(), "loadtest lease was not released");
    }
    result
}

async fn take(chat: &ChatState, token: &str) -> ChatResult<bool> {
    let lease_ms = u64::try_from(LEASE.as_millis()).unwrap_or(u64::MAX);
    let mut conn = chat.redis.get().await?;
    let taken: Option<String> = redis::cmd("SET")
        .arg(rt_keys::LOADTEST_BUSY)
        .arg(token)
        .arg("NX")
        .arg("PX")
        .arg(lease_ms)
        .query_async(&mut conn)
        .await?;
    Ok(taken.is_some())
}

async fn release(chat: &ChatState, token: &str) -> ChatResult<()> {
    let mut conn = chat.redis.get().await?;
    let _: i64 = RELEASE
        .key(rt_keys::LOADTEST_BUSY)
        .arg(token)
        .invoke_async(&mut conn)
        .await?;
    Ok(())
}
