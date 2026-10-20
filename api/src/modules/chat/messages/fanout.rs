use std::sync::LazyLock;

use redis::Script;
use uuid::Uuid;

use crate::modules::chat::model::MessageRow;
use crate::modules::chat::rt_keys::{self, DELTAS_KEPT, VERSION_TTL};
use crate::modules::chat::{ChatResult, ChatState};

const BUMP_SCRIPT: &str = r"
local seq = tonumber(ARGV[1])
local ttl = tonumber(ARGV[3])
local kept = tonumber(ARGV[4])
local current = tonumber(redis.call('GET', KEYS[1]) or '-1')
if seq > current then
  redis.call('SET', KEYS[1], seq, 'EX', ttl)
else
  redis.call('EXPIRE', KEYS[1], ttl)
end
for i = 2, #KEYS, 2 do
  local version = redis.call('INCR', KEYS[i])
  redis.call('EXPIRE', KEYS[i], ttl)
  redis.call('ZADD', KEYS[i + 1], version, ARGV[2])
  redis.call('ZREMRANGEBYRANK', KEYS[i + 1], 0, -(kept + 1))
  redis.call('EXPIRE', KEYS[i + 1], ttl)
end
return (#KEYS - 1) / 2
";

static BUMP: LazyLock<Script> = LazyLock::new(|| Script::new(BUMP_SCRIPT));

pub async fn publish(
    chat: &ChatState,
    rows: &[MessageRow],
    recipients: &[String],
) -> ChatResult<()> {
    let Some(last) = rows.iter().max_by_key(|row| row.seq) else {
        return Ok(());
    };
    for row in rows {
        chat.hot.put(row.clone());
    }
    bump(chat, last.conversation_id, last.seq, recipients).await
}

pub async fn bump(
    chat: &ChatState,
    conversation: Uuid,
    seq: i64,
    recipients: &[String],
) -> ChatResult<()> {
    if recipients.is_empty() {
        return Ok(());
    }
    let mut invocation = BUMP.prepare_invoke();
    invocation.key(rt_keys::conversation_seq(conversation));
    for user in recipients {
        invocation.key(rt_keys::version(user));
        invocation.key(rt_keys::deltas(user));
    }
    invocation
        .arg(seq)
        .arg(conversation.to_string())
        .arg(VERSION_TTL.as_secs())
        .arg(DELTAS_KEPT);
    let mut conn = chat.redis.get().await?;
    let _: i64 = invocation.invoke_async(&mut conn).await?;
    drop(conn);
    for user in recipients {
        chat.hub.wake(user);
    }
    Ok(())
}

#[cfg(test)]
#[path = "../tests/fanout_tests.rs"]
mod tests;
