use redis::Script;
use uuid::Uuid;

use crate::modules::chat::rt_keys::{self, DELTAS_KEPT, VERSION_TTL};
use crate::modules::chat::{ChatResult, ChatState};

const BUMP_SCRIPT: &str = r"
local seq = tonumber(ARGV[2])
if seq > 0 then
  local current = tonumber(redis.call('GET', KEYS[1]) or '0')
  if seq > current then redis.call('SET', KEYS[1], seq, 'EX', ARGV[3]) end
end
for i = 2, #KEYS, 2 do
  local version = redis.call('INCR', KEYS[i])
  redis.call('EXPIRE', KEYS[i], ARGV[3])
  redis.call('ZADD', KEYS[i + 1], version, ARGV[1])
  redis.call('ZREMRANGEBYRANK', KEYS[i + 1], 0, -(tonumber(ARGV[4]) + 1))
  redis.call('EXPIRE', KEYS[i + 1], ARGV[3])
end
return 1
";

pub async fn bump(
    chat: &ChatState,
    conversation: Uuid,
    seq: i64,
    users: &[String],
) -> ChatResult<()> {
    let script = Script::new(BUMP_SCRIPT);
    let mut call = script.prepare_invoke();
    call.key(rt_keys::conversation_seq(conversation));
    for user in users {
        call.key(rt_keys::version(user)).key(rt_keys::deltas(user));
    }
    call.arg(conversation.to_string())
        .arg(seq.max(0))
        .arg(VERSION_TTL.as_secs())
        .arg(DELTAS_KEPT);
    let mut conn = chat.redis.get().await?;
    call.invoke_async::<i64>(&mut conn).await?;
    for user in users {
        chat.hub.wake(user);
    }
    Ok(())
}
