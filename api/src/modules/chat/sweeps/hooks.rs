use std::sync::LazyLock;

use redis::Script;
use uuid::Uuid;

use crate::error::AppError;
use crate::modules::chat::sweeps::purge;
use crate::modules::chat::{ChatError, ChatResult, ChatState, rt_keys};
use crate::modules::rooms::model::Room;
use crate::modules::rooms::store::RoomStore;

pub const RECIPIENTS_PER_CALL: usize = 256;
pub const GONE_SEQS_PER_ENTRY: usize = 64;

const DEVICES_CHANGE: &str = "devices";

const BUMP_SCRIPT: &str = r"
local ttl = tonumber(ARGV[1])
local kept = tonumber(ARGV[2])
local seq = tonumber(ARGV[3])
local first = 1
if seq > 0 then
  first = 2
  if seq > tonumber(redis.call('GET', KEYS[1]) or '0') then
    redis.call('SET', KEYS[1], ARGV[3], 'EX', ttl)
  else
    redis.call('EXPIRE', KEYS[1], ttl)
  end
end
local clock = redis.call('TIME')
local now = tonumber(clock[1]) * 1000 + math.floor(tonumber(clock[2]) / 1000)
for index = first, #KEYS, 2 do
  local stored = redis.call('HMGET', KEYS[index], 'v', 'f')
  local version = tonumber(stored[1]) or now
  local floor = tonumber(stored[2]) or version
  version = math.max(version + 1, now)
  local score = string.format('%d', version)
  for member = 4, #ARGV do
    redis.call('ZADD', KEYS[index + 1], score, ARGV[member])
  end
  local excess = redis.call('ZCARD', KEYS[index + 1]) - kept
  if excess > 0 then
    local dropped = redis.call('ZRANGE', KEYS[index + 1], excess - 1, excess - 1, 'WITHSCORES')
    floor = math.max(floor, tonumber(dropped[2]))
    redis.call('ZREMRANGEBYRANK', KEYS[index + 1], 0, excess - 1)
  end
  redis.call('HSET', KEYS[index], 'v', score, 'f', string.format('%d', floor))
  redis.call('EXPIRE', KEYS[index], ttl)
  redis.call('EXPIRE', KEYS[index + 1], ttl)
end
return 1
";

static BUMP: LazyLock<Script> = LazyLock::new(|| Script::new(BUMP_SCRIPT));

pub fn gone_member(conversation: Uuid, seqs: &[i64]) -> String {
    let seqs: Vec<String> = seqs.iter().map(i64::to_string).collect();
    format!("gone:{conversation}:{}", seqs.join(","))
}

pub async fn bump(
    chat: &ChatState,
    conversation: Uuid,
    last_seq: i64,
    recipients: &[String],
) -> ChatResult<()> {
    let members = [conversation.to_string()];
    deltas(chat, Some((conversation, last_seq)), &members, recipients).await
}

pub async fn messages_gone(
    chat: &ChatState,
    conversation: Uuid,
    seqs: &[i64],
    recipients: &[String],
) -> ChatResult<()> {
    let members: Vec<String> = seqs
        .chunks(GONE_SEQS_PER_ENTRY)
        .map(|chunk| gone_member(conversation, chunk))
        .collect();
    deltas(chat, None, &members, recipients).await
}

pub async fn devices_revoked(chat: &ChatState, user_id: &str, ids: &[Uuid]) -> ChatResult<()> {
    chat.devices.forget(user_id, ids);
    let members = [DEVICES_CHANGE.to_owned()];
    deltas(chat, None, &members, &[user_id.to_owned()]).await
}

pub async fn room_of_chat(chat: &ChatState, conversation: Uuid) -> ChatResult<Option<Room>> {
    let code = sqlx::query_file_scalar!("queries/chat/sweeps/room_ref.sql", conversation)
        .fetch_optional(&chat.pg)
        .await?
        .flatten();
    let Some(code) = code else {
        return Ok(None);
    };
    RoomStore::new(chat.redis.clone())
        .load(&code)
        .await
        .map_err(room_failure)
}

pub async fn room_seen_since(chat: &ChatState, code: &str, since_ms: i64) -> ChatResult<bool> {
    let seen = RoomStore::new(chat.redis.clone())
        .seen_since(code, since_ms)
        .await
        .map_err(room_failure)?;
    Ok(!seen.is_empty())
}

pub async fn end_room(chat: &ChatState, code: &str) -> ChatResult<bool> {
    let store = RoomStore::new(chat.redis.clone());
    let Some(room) = store.load(code).await.map_err(room_failure)? else {
        return Ok(false);
    };
    store.delete(code).await.map_err(room_failure)?;
    let mut conn = chat.redis.get().await?;
    let _: i64 = redis::cmd("ZREM")
        .arg(rt_keys::ROOMS_ALIVE)
        .arg(code)
        .query_async(&mut conn)
        .await?;
    drop(conn);
    let members: Vec<String> = room.members.iter().map(|m| m.user_id.clone()).collect();
    let chats = sqlx::query_file_scalar!("queries/chat/sweeps/room_chats.sql", code)
        .fetch_all(&chat.pg)
        .await?;
    for conversation in chats {
        if purge::shred(chat, conversation).await? {
            bump(chat, conversation, 0, &members).await?;
        }
    }
    Ok(true)
}

async fn deltas(
    chat: &ChatState,
    seq: Option<(Uuid, i64)>,
    members: &[String],
    recipients: &[String],
) -> ChatResult<()> {
    let mut recipients = recipients.to_vec();
    recipients.sort();
    recipients.dedup();
    let seq = seq.filter(|(_, last_seq)| *last_seq > 0);
    if members.is_empty() || (recipients.is_empty() && seq.is_none()) {
        return Ok(());
    }
    let mut batches: Vec<&[String]> = recipients.chunks(RECIPIENTS_PER_CALL).collect();
    if batches.is_empty() {
        batches.push(&[]);
    }
    let mut conn = chat.redis.get().await?;
    for (index, batch) in batches.into_iter().enumerate() {
        let batch_seq = seq.filter(|_| index == 0);
        let mut call = BUMP.prepare_invoke();
        if let Some((conversation, _)) = batch_seq {
            call.key(rt_keys::conversation_seq(conversation));
        }
        for recipient in batch {
            call.key(rt_keys::version(recipient))
                .key(rt_keys::deltas(recipient));
        }
        call.arg(rt_keys::VERSION_TTL.as_secs())
            .arg(rt_keys::DELTAS_KEPT)
            .arg(batch_seq.map_or(0, |(_, last_seq)| last_seq))
            .arg(members);
        let _: i64 = call.invoke_async(&mut conn).await?;
    }
    drop(conn);
    for recipient in &recipients {
        chat.hub.wake(recipient);
    }
    Ok(())
}

fn room_failure(error: AppError) -> ChatError {
    tracing::warn!(?error, "a chat sweep could not read a room");
    ChatError::Busy { retry_ms: 1_000 }
}
